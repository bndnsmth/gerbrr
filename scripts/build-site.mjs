// Package an explicit allowlist, never the development tree or fabrication cache.
import { createHash } from "node:crypto";
import { lstat, mkdir, mkdtemp, readFile, rename, rm, writeFile } from "node:fs/promises";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const project = fileURLToPath(new URL("../", import.meta.url));
const marker = ".gerbrr-site.json";
const assetLimit = 25 * 1024 * 1024;
const versioned = ["main.js", "worker.js", "style.css", "icon.svg", "pkg/gerbrr.js", "pkg/gerbrr_bg.wasm"];
const roots = [
  ["web/_headers", "_headers"],
  ["web/robots.txt", "robots.txt"],
  ["assets/preview.png", "preview.png"],
  ["LICENSE", "licenses/gerbrr.txt"],
  ["assets/octicons-LICENSE.txt", "licenses/octicons.txt"],
];

async function source(root, path) {
  const file = join(root, path);
  if (!(await lstat(file)).isFile()) throw new Error(`Expected a regular public asset: ${path}`);
  const data = await readFile(file);
  if (!data.length || data.length > assetLimit) throw new Error(`Asset must be nonempty and at most 25 MiB: ${path}`);
  return data;
}

async function exists(path) {
  try { return await lstat(path); } catch (error) { if (error.code === "ENOENT") return null; throw error; }
}

export async function buildSite({ root = project } = {}) {
  root = resolve(root);
  const output = join(root, "dist");
  const old = await exists(output);
  if (old) {
    if (!old.isDirectory()) throw new Error("Refusing to replace a non-directory or symlink at dist");
    let owned;
    try { owned = JSON.parse(await readFile(join(output, marker), "utf8")); } catch {}
    if (owned?.app !== "gerbrr" || owned?.format !== 1) throw new Error("Refusing to replace an unrecognized dist directory");
  }

  // Finish validating every input before touching an existing production build.
  const inputs = new Map();
  inputs.set("web/index.html", await source(root, "web/index.html"));
  for (const path of versioned) inputs.set(`web/${path}`, await source(root, `web/${path}`));
  for (const [path] of roots) inputs.set(path, await source(root, path));
  if (!inputs.get("web/pkg/gerbrr_bg.wasm").subarray(0, 8).equals(Buffer.from([0, 97, 115, 109, 1, 0, 0, 0]))) {
    throw new Error("The browser build is not a valid WASM module");
  }
  const hash = createHash("sha256");
  for (const [path, data] of [...inputs].sort(([a], [b]) => a.localeCompare(b))) {
    hash.update(path).update("\0").update(data).update("\0");
  }
  const release = hash.digest("hex").slice(0, 20);
  const prefix = `/assets/${release}/`;
  let html = inputs.get("web/index.html").toString("utf8");
  for (const path of ["main.js", "style.css", "icon.svg"]) {
    if (!html.includes(`"./${path}"`)) throw new Error(`Missing expected shell reference: ${path}`);
    html = html.replaceAll(`"./${path}"`, `"${prefix}${path}"`);
  }
  const files = new Map([["index.html", Buffer.from(html)]]);
  for (const path of versioned) files.set(`assets/${release}/${path}`, inputs.get(`web/${path}`));
  for (const [from, to] of roots) files.set(to, inputs.get(from));
  files.set(".assetsignore", Buffer.from(`${marker}\n`));
  const manifest = { app: "gerbrr", format: 1, release, files: [...files.keys()].sort() };

  const stage = await mkdtemp(join(root, ".gerbrr-site-"));
  const backup = `${stage}-previous`;
  let replaced = false;
  try {
    for (const [path, data] of files) {
      await mkdir(dirname(join(stage, path)), { recursive: true });
      await writeFile(join(stage, path), data);
    }
    await writeFile(join(stage, marker), `${JSON.stringify(manifest, null, 2)}\n`);
    if (old) { await rename(output, backup); replaced = true; }
    try { await rename(stage, output); }
    catch (error) { if (replaced) await rename(backup, output); throw error; }
    if (replaced) await rm(backup, { recursive: true });
  } finally {
    // Only this invocation's generated staging directory is eligible for cleanup.
    if (await exists(stage)) await rm(stage, { recursive: true });
  }
  return manifest;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 2) throw new Error(`Usage: node ${basename(process.argv[1])}`);
  const build = await buildSite();
  console.log(`Gerbrr production build: dist/ · release ${build.release} · ${build.files.length} public assets`);
}
