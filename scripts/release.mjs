// Native release tooling. No deployment credentials or fabrication files are used.
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { appendFile, chmod, copyFile, lstat, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const project = fileURLToPath(new URL("../", import.meta.url));
export const targets = {
  "x86_64-unknown-linux-gnu": { platform: "linux", arch: "x64", label: "linux-x86_64", extension: ".tar.gz", binary: "gerbrr" },
  "x86_64-pc-windows-msvc": { platform: "win32", arch: "x64", label: "windows-x86_64", extension: ".zip", binary: "gerbrr.exe" },
  "aarch64-apple-darwin": { platform: "darwin", arch: "arm64", label: "macos-arm64", extension: ".zip", binary: "gerbrr" },
  "x86_64-apple-darwin": { platform: "darwin", arch: "x64", label: "macos-x86_64", extension: ".zip", binary: "gerbrr" },
};
const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

export function releaseInfo(toml, tag = "") {
  const section = toml.match(/^\[package\]\s*\n([\s\S]*?)(?=^\[|$(?![\s\S]))/m)?.[1];
  const name = section?.match(/^name\s*=\s*"([^"]+)"/m)?.[1];
  const version = section?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if (name !== "gerbrr" || !version || !semver.test(version)) throw new Error("Expected gerbrr's explicit SemVer version in Cargo.toml");
  const expected = `v${version}`;
  if (tag && tag !== expected) throw new Error(`Tag ${tag} does not match Cargo.toml version ${version}; expected ${expected}`);
  return { version, tag: expected, prerelease: Boolean(version.match(semver)[4]) };
}

export function packageName(info, target) {
  if (!targets[target]) throw new Error(`Unsupported release target: ${target}`);
  if (!semver.test(info.version) || info.tag !== `v${info.version}`) throw new Error("Invalid release version/tag");
  return `gerbrr-${info.tag}-${targets[target].label}`;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: "utf8", maxBuffer: 32 * 1024 * 1024, ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed (${result.status}): ${result.stderr || result.stdout}`);
  return result.stdout.trim();
}

async function regularFile(path) {
  if (!(await lstat(path)).isFile()) throw new Error(`Expected a regular release input: ${basename(path)}`);
  if (!(await readFile(path)).length) throw new Error(`Empty release input: ${basename(path)}`);
}

export function macPlist(info) {
  if (!semver.test(info.version)) throw new Error("Invalid app version");
  const version = info.version.split(/[+-]/)[0];
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Gerbrr</string>
  <key>CFBundleDisplayName</key><string>Gerbrr</string>
  <key>CFBundleIdentifier</key><string>org.gerbrr.app</string>
  <key>CFBundleExecutable</key><string>gerbrr</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>gerbrr.icns</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict></plist>\n`;
}

function instructions(info, target) {
  const os = targets[target].platform;
  const launch = os === "darwin" ? "Move Gerbrr.app to Applications, then open it.\nCLI: ./Gerbrr.app/Contents/MacOS/gerbrr inspect board.zip"
    : os === "win32" ? "Open gerbrr.exe.\nPowerShell CLI: .\\gerbrr.exe inspect board.zip"
      : "Run ./gerbrr.\nCLI: ./gerbrr inspect board.zip\nRequires glibc 2.35+ and desktop graphics libraries (Wayland/X11, xkbcommon, and a Vulkan/OpenGL driver).";
  return `Gerbrr ${info.version} — Gerber viewing at ridiculous speed.\n\n${launch}
\nOpen board accepts files, folders, or ZIPs. No arguments opens the Atlas demo.
Board files stay local. This is an artwork viewer, not manufacturing DRC.
\nWindows builds are unsigned. macOS bundles are ad-hoc signed, not Developer ID
signed or notarized. Your OS may show a security warning; review the release and
use the OS's normal approval flow. Do not disable system-wide security settings.
\nCode and third-party license notices are included.\nhttps://gerbrr.bndnsmth.com/\n`;
}

export async function packageRelease({ root = project, target, tag = "", platform = process.platform,
  arch = process.arch, execute = run } = {}) {
  const info = releaseInfo(await readFile(join(root, "Cargo.toml"), "utf8"), tag);
  const name = packageName(info, target);
  const spec = targets[target];
  if (platform !== spec.platform || arch !== spec.arch) throw new Error(`Build/package ${target} on its native OS and architecture`);
  const binary = join(root, "target", target, "release", spec.binary);
  const notices = join(root, "target", "release-notices", `${target}.html`);
  for (const path of [binary, notices, join(root, "LICENSE"), join(root, "assets/octicons-LICENSE.txt")]) await regularFile(path);
  if (execute(binary, ["--version"]) !== `gerbrr ${info.version}`) throw new Error("Built binary version does not match the release");
  if (!execute(binary, ["inspect"]).includes("6 layers")) throw new Error("The built-in demo failed its binary smoke test");
  const output = join(root, "target", "release-artifacts");
  await mkdir(output, { recursive: true });
  const archive = join(output, `${name}${spec.extension}`);
  try { await lstat(archive); throw new Error(`Refusing to overwrite ${basename(archive)}; use a fresh build directory`); }
  catch (error) { if (error.code !== "ENOENT") throw error; }
  const stage = await mkdtemp(join(root, "target/release-package-"));
  try {
    const folder = join(stage, name);
    await mkdir(join(folder, "licenses"), { recursive: true });
    await copyFile(join(root, "LICENSE"), join(folder, "LICENSE"));
    await copyFile(join(root, "assets/octicons-LICENSE.txt"), join(folder, "licenses/octicons-LICENSE.txt"));
    await copyFile(notices, join(folder, "licenses/THIRD_PARTY_LICENSES.html"));
    await writeFile(join(folder, "README.txt"), instructions(info, target));
    if (spec.platform === "darwin") {
      await regularFile(join(root, "assets/gerbrr.png"));
      const app = join(folder, "Gerbrr.app");
      const contents = join(app, "Contents");
      await mkdir(join(contents, "MacOS"), { recursive: true });
      await mkdir(join(contents, "Resources"));
      await copyFile(binary, join(contents, "MacOS/gerbrr"));
      await chmod(join(contents, "MacOS/gerbrr"), 0o755);
      await writeFile(join(contents, "Info.plist"), macPlist(info));
      const iconset = join(stage, "gerbrr.iconset");
      await mkdir(iconset);
      for (const size of [16, 32, 128, 256, 512]) {
        for (const scale of [1, 2]) execute("sips", ["-z", String(size * scale), String(size * scale),
          join(root, "assets/gerbrr.png"), "--out", join(iconset, `icon_${size}x${size}${scale === 2 ? "@2x" : ""}.png`)]);
      }
      execute("iconutil", ["-c", "icns", iconset, "-o", join(contents, "Resources/gerbrr.icns")]);
      execute("codesign", ["--force", "--deep", "--sign", "-", app]);
      execute("codesign", ["--verify", "--deep", "--strict", app]);
      execute("ditto", ["-c", "-k", "--sequesterRsrc", "--keepParent", folder, archive]);
    } else {
      await copyFile(binary, join(folder, spec.binary));
      if (spec.platform === "linux") {
        await chmod(join(folder, spec.binary), 0o755);
        execute("tar", ["--sort=name", "--owner=0", "--group=0", "--numeric-owner", "-czf", archive, "-C", stage, name]);
      } else {
        execute("pwsh", ["-NoLogo", "-NoProfile", "-Command",
          "$ErrorActionPreference = 'Stop'; Compress-Archive -LiteralPath $env:GERBRR_PACKAGE_SOURCE -DestinationPath $env:GERBRR_PACKAGE_ARCHIVE -CompressionLevel Optimal"],
        { env: { ...process.env, GERBRR_PACKAGE_SOURCE: folder, GERBRR_PACKAGE_ARCHIVE: archive } });
      }
    }
    await regularFile(archive);
    const hash = createHash("sha256").update(await readFile(archive)).digest("hex");
    await writeFile(`${archive}.sha256`, `${hash}  ${basename(archive)}\n`);
    return { ...info, target, archive, hash };
  } finally { await rm(stage, { recursive: true }); }
}

export async function releaseChecksums({ root = project, directory = join(root, "target/release-artifacts"), tag = "" } = {}) {
  const info = releaseInfo(await readFile(join(root, "Cargo.toml"), "utf8"), tag);
  const names = Object.keys(targets).map(target => `${packageName(info, target)}${targets[target].extension}`).sort();
  const allowed = new Set([...names, ...names.map(name => `${name}.sha256`), "SHA256SUMS"]);
  if ((await readdir(directory)).some(name => !allowed.has(name))) throw new Error("Unexpected release file; refusing to upload it");
  let sums = "";
  for (const name of names) {
    await regularFile(join(directory, name));
    const hash = createHash("sha256").update(await readFile(join(directory, name))).digest("hex");
    const expected = `${hash}  ${name}\n`;
    if (await readFile(join(directory, `${name}.sha256`), "utf8") !== expected) throw new Error(`Checksum mismatch for ${name}`);
    sums += expected;
  }
  await writeFile(join(directory, "SHA256SUMS"), sums);
  return { ...info, files: [...names.map(name => join(directory, name)), join(directory, "SHA256SUMS")] };
}

export async function publishRelease() {
  const info = await releaseChecksums({ tag: process.env.RELEASE_TAG ?? "" });
  const repository = process.env.GITHUB_REPOSITORY;
  if (!repository || !process.env.GITHUB_SHA || !process.env.GH_TOKEN) throw new Error("Publish only inside the tag workflow with its scoped GitHub token");
  // HEAD is the checked-out commit even when the event SHA is an annotated tag object.
  const commit = run("git", ["rev-parse", "HEAD"], { cwd: project });
  const remote = run("gh", ["api", `repos/${repository}/commits/${info.tag}`, "--jq", ".sha"]);
  if (remote !== commit) throw new Error("The release tag moved after this build; refusing to publish stale binaries");
  const existing = spawnSync("gh", ["release", "view", info.tag, "--repo", repository, "--json", "isDraft"], { encoding: "utf8" });
  if (existing.status === 0) {
    if (!JSON.parse(existing.stdout).isDraft) throw new Error("Refusing to overwrite an already-published release");
    run("gh", ["release", "upload", info.tag, "--repo", repository, "--clobber", ...info.files]);
  } else {
    const notes = "Native downloads: Linux x86-64, Windows x86-64, and macOS Apple Silicon/Intel.\n\n"
      + "Windows builds are unsigned. macOS apps are ad-hoc signed, not Developer ID signed or notarized; OS security warnings may appear. "
      + "Linux requires glibc 2.35+; macOS requires 13+. A compatible desktop graphics driver is required.\n\n"
      + "SHA256SUMS and license notices are included. Extract your platform's archive and follow its README.txt.\n\n";
    run("gh", ["release", "create", info.tag, "--repo", repository, "--verify-tag", "--draft",
      "--title", `Gerbrr ${info.version}`, "--generate-notes", "--notes", notes,
      ...(info.prerelease ? ["--prerelease"] : []), ...info.files]);
  }
  console.log(`Draft release ready: ${info.tag}. Review the downloads, then publish from GitHub Releases.`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const command = process.argv[2];
  const info = releaseInfo(await readFile(join(project, "Cargo.toml"), "utf8"), process.env.RELEASE_TAG ?? "");
  if (command === "info") {
    if (process.env.GITHUB_OUTPUT) await appendFile(process.env.GITHUB_OUTPUT,
      `version=${info.version}\ntag=${info.tag}\nprerelease=${info.prerelease}\n`);
    console.log(JSON.stringify(info));
  } else if (command === "package" && process.argv.length === 4) {
    console.log(JSON.stringify(await packageRelease({ target: process.argv[3], tag: info.tag })));
  } else if (command === "publish" && process.argv.length === 3) {
    await publishRelease();
  } else {
    throw new Error("Usage: node scripts/release.mjs info | package TARGET | publish");
  }
}
