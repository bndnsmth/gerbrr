import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { chmod, mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { macPlist, packageName, packageRelease, releaseChecksums, releaseInfo, targets } from "../scripts/release.mjs";

const project = fileURLToPath(new URL("../", import.meta.url));
const manifest = '[package]\nname = "gerbrr"\nversion = "0.1.0"\n\n[dependencies]\n';
const info = releaseInfo(manifest);

async function fixture(t) {
  await mkdir(join(project, "target"), { recursive: true });
  const root = await mkdtemp(join(project, "target/release-test-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  for (const [path, contents] of Object.entries({
    "Cargo.toml": manifest, "LICENSE": "Project license", "assets/octicons-LICENSE.txt": "Icon license",
    "assets/gerbrr.png": "Icon PNG", ".env.local": "DO NOT PACKAGE ACCOUNT SETTINGS",
    "target/corpus/private.gbr": "DO NOT PACKAGE FABRICATION FILES",
    "target/release-notices/x86_64-unknown-linux-gnu.html": "<html>Dependency license texts</html>",
  })) {
    await mkdir(join(root, path, ".."), { recursive: true });
    await writeFile(join(root, path), contents);
  }
  return root;
}

test("release tag must exactly match the Cargo package version", () => {
  assert.deepEqual(info, { version: "0.1.0", tag: "v0.1.0", prerelease: false });
  assert.deepEqual(releaseInfo(manifest, "v0.1.0"), info);
  assert.throws(() => releaseInfo(manifest, "v0.2.0"), /does not match/);
  assert.throws(() => releaseInfo(manifest, "0.1.0"), /does not match/);
  assert.throws(() => releaseInfo('[workspace]\nversion = "0.1.0"'), /explicit SemVer/);
  assert.throws(() => releaseInfo(manifest.replace('"gerbrr"', '"other"')), /gerbrr/);
});

test("SemVer prereleases/build metadata work and unsafe versions are rejected", () => {
  for (const version of ["1.2.3-rc.1", "1.2.3-beta", "1.2.3+build.7", "1.2.3-rc.1+build.7"]) {
    const result = releaseInfo(manifest.replace("0.1.0", version), `v${version}`);
    assert.equal(result.version, version);
    assert.equal(result.prerelease, version.includes("-"));
  }
  for (const version of ["01.2.3", "1.2", "1.2.3-01", "../secret", "1.2.3/other", "1.2.3\nother", "1.2.3;command"]) {
    assert.throws(() => releaseInfo(manifest.replace("0.1.0", version)));
  }
});

test("four distinct native packages have safe versioned names", () => {
  assert.equal(Object.keys(targets).length, 4);
  assert.equal(packageName(info, "x86_64-unknown-linux-gnu"), "gerbrr-v0.1.0-linux-x86_64");
  assert.equal(packageName(info, "x86_64-pc-windows-msvc"), "gerbrr-v0.1.0-windows-x86_64");
  assert.equal(packageName(info, "aarch64-apple-darwin"), "gerbrr-v0.1.0-macos-arm64");
  assert.equal(packageName(info, "x86_64-apple-darwin"), "gerbrr-v0.1.0-macos-x86_64");
  assert.throws(() => packageName(info, "../../wrong"));
  assert.throws(() => packageName({ ...info, tag: "other" }, "aarch64-apple-darwin"));
});

test("macOS bundles use a generic ID and numeric bundle versions", () => {
  const plist = macPlist(releaseInfo(manifest.replace("0.1.0", "1.2.3-rc.1")));
  assert.match(plist, /org\.gerbrr\.app/);
  assert.match(plist, /<key>CFBundleExecutable<\/key><string>gerbrr<\/string>/);
  assert.match(plist, /<key>CFBundleVersion<\/key><string>1\.2\.3<\/string>/);
  assert.ok(!plist.includes("rc.1"));
});

test("release packaging rejects a non-native OS/architecture before running a binary", async (t) => {
  const root = await fixture(t);
  await assert.rejects(packageRelease({ root, target: "aarch64-apple-darwin", platform: "linux", arch: "x64" }), /native OS/);
});

test("Linux archive contains only the executable, usage notes, and licenses", { skip: process.platform !== "linux" }, async (t) => {
  const root = await fixture(t);
  const binary = join(root, "target/x86_64-unknown-linux-gnu/release/gerbrr");
  await mkdir(join(binary, ".."), { recursive: true });
  await writeFile(binary, '#!/bin/sh\ncase "$1" in --version) echo "gerbrr 0.1.0";; inspect) echo "6 layers";; *) exit 1;; esac\n');
  await chmod(binary, 0o755);
  const result = await packageRelease({ root, target: "x86_64-unknown-linux-gnu", arch: "x64" });
  const listing = spawnSync("tar", ["-tzf", result.archive], { encoding: "utf8" });
  assert.equal(listing.status, 0);
  const files = listing.stdout.trim().split("\n").filter(path => !path.endsWith("/")).map(path => path.split("/").slice(1).join("/")).sort();
  assert.deepEqual(files, ["LICENSE", "README.txt", "gerbrr", "licenses/THIRD_PARTY_LICENSES.html", "licenses/octicons-LICENSE.txt"].sort());
  assert.equal(await readFile(`${result.archive}.sha256`, "utf8"), `${result.hash}  ${result.archive.split("/").at(-1)}\n`);
  assert.ok((await readdir(join(root, "target"))).every(path => !path.startsWith("release-package-")));
  await assert.rejects(packageRelease({ root, target: "x86_64-unknown-linux-gnu", arch: "x64" }), /overwrite/);
});

test("missing license notices and symlinked binary inputs are rejected", async (t) => {
  const root = await fixture(t);
  const binary = join(root, "target/x86_64-unknown-linux-gnu/release/gerbrr");
  await mkdir(join(binary, ".."), { recursive: true });
  if (process.platform !== "win32") {
    await symlink(join(root, ".env.local"), binary);
    await assert.rejects(packageRelease({ root, target: "x86_64-unknown-linux-gnu", platform: "linux", arch: "x64" }), /regular release input/);
    await rm(binary);
  }
  await writeFile(binary, "binary fixture");
  const notices = join(root, "target/release-notices/x86_64-unknown-linux-gnu.html");
  await rm(notices);
  await assert.rejects(packageRelease({ root, target: "x86_64-unknown-linux-gnu", platform: "linux", arch: "x64" }), /ENOENT/);
  await writeFile(notices, "");
  await assert.rejects(packageRelease({ root, target: "x86_64-unknown-linux-gnu", platform: "linux", arch: "x64" }), /Empty release input/);
});

test("Windows packaging uses native ZIP tooling and an explicit file allowlist", async (t) => {
  const root = await fixture(t);
  const target = "x86_64-pc-windows-msvc";
  const binary = join(root, `target/${target}/release/gerbrr.exe`);
  await mkdir(join(binary, ".."), { recursive: true });
  await writeFile(binary, "Windows executable fixture");
  await writeFile(join(root, `target/release-notices/${target}.html`), "License notices");
  let zip = false;
  const result = await packageRelease({ root, target, platform: "win32", arch: "x64", execute(command, args, options) {
    if (command === binary) return args[0] === "--version" ? "gerbrr 0.1.0" : "6 layers";
    assert.equal(command, "pwsh");
    assert.ok(args.at(-1).includes("Compress-Archive -LiteralPath"));
    const folder = options.env.GERBRR_PACKAGE_SOURCE;
    assert.deepEqual(readdirSync(folder).sort(), ["LICENSE", "README.txt", "gerbrr.exe", "licenses"].sort());
    assert.equal(readFileSync(join(folder, "gerbrr.exe"), "utf8"), "Windows executable fixture");
    writeFileSync(options.env.GERBRR_PACKAGE_ARCHIVE, "ZIP fixture");
    zip = true;
    return "";
  } });
  assert.ok(zip && result.archive.endsWith("windows-x86_64.zip"));
});

test("macOS packaging builds the app, icon, and ad-hoc signature before archiving", async (t) => {
  const root = await fixture(t);
  const target = "aarch64-apple-darwin";
  const binary = join(root, `target/${target}/release/gerbrr`);
  await mkdir(join(binary, ".."), { recursive: true });
  await writeFile(binary, "Mac executable fixture");
  await writeFile(join(root, `target/release-notices/${target}.html`), "License notices");
  const commands = [];
  await packageRelease({ root, target, platform: "darwin", arch: "arm64", execute(command, args) {
    if (command === binary) return args[0] === "--version" ? "gerbrr 0.1.0" : "6 layers";
    commands.push([command, args]);
    if (command === "iconutil") writeFileSync(args.at(-1), "ICNS fixture");
    if (command === "ditto") {
      const folder = args.at(-2);
      assert.deepEqual(readdirSync(folder).sort(), ["Gerbrr.app", "LICENSE", "README.txt", "licenses"].sort());
      const executable = join(folder, "Gerbrr.app/Contents/MacOS/gerbrr");
      assert.equal(readFileSync(executable, "utf8"), "Mac executable fixture");
      if (process.platform !== "win32") assert.equal(statSync(executable).mode & 0o777, 0o755);
      assert.match(readFileSync(join(folder, "Gerbrr.app/Contents/Info.plist"), "utf8"), /org\.gerbrr\.app/);
      writeFileSync(args.at(-1), "ZIP fixture");
    }
    return "";
  } });
  assert.equal(commands.filter(([command]) => command === "sips").length, 10);
  assert.deepEqual(commands.slice(-3).map(([command]) => command), ["codesign", "codesign", "ditto"]);
  assert.ok(commands.some(([command, args]) => command === "codesign" && args.includes("--sign") && args.includes("-")));
});

test("combined checksums require all four exact packages and reject tampering or extra files", async (t) => {
  const root = await fixture(t);
  const directory = join(root, "target/release-artifacts");
  await mkdir(directory);
  await assert.rejects(releaseChecksums({ root }), /ENOENT/);
  const paths = [];
  for (const [target, spec] of Object.entries(targets)) {
    const name = `${packageName(info, target)}${spec.extension}`;
    const body = Buffer.from(`Artifact for ${target}`);
    const hash = createHash("sha256").update(body).digest("hex");
    await writeFile(join(directory, name), body);
    await writeFile(join(directory, `${name}.sha256`), `${hash}  ${name}\n`);
    paths.push(name);
  }
  const result = await releaseChecksums({ root });
  assert.equal(result.files.length, 5);
  assert.equal((await readFile(join(directory, "SHA256SUMS"), "utf8")).trim().split("\n").length, 4);
  await writeFile(join(directory, paths[0]), "modified");
  await assert.rejects(releaseChecksums({ root }), /Checksum mismatch/);
  await writeFile(join(directory, "unexpected.env"), "PRIVATE");
  await assert.rejects(releaseChecksums({ root }), /Unexpected release file/);
});
