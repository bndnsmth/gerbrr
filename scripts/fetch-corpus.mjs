// Fetch immutable public fabrication inputs. Never runs upstream scripts or extracts ZIPs.
import { createHash } from "node:crypto";
import { link, lstat, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const manifest = JSON.parse(await readFile(join(root, "tests/fixtures/corpus.json"), "utf8"));
if (manifest.schema !== 1 || !Array.isArray(manifest.samples)) throw new Error("Unsupported corpus manifest");
const requested = new Set(process.argv.slice(2));
for (const id of requested) {
  if (!manifest.samples.some((sample) => sample.id === id)) throw new Error(`Unknown corpus ID: ${id}`);
}

function verify(bytes, file, label) {
  if (bytes.length !== file.bytes || createHash("sha256").update(bytes).digest("hex") !== file.sha256) {
    throw new Error(`${label}: size/SHA-256 mismatch. Cached files are not overwritten; investigate this exact file.`);
  }
}

async function fetchFile(sample, file) {
  if (!/^[a-z0-9-]+$/.test(sample.id) || !/^[a-f0-9]{40}$/.test(sample.commit)
    || !/^[\w.-]+\/[\w.-]+$/.test(sample.repo)
    || !["input", "provenance"].includes(file.role)
    || file.source.split("/").some((part) => !part || part === ".." || part === ".")
    || file.source.includes("\\") || !/^[a-f0-9]{64}$/.test(file.sha256)
    || !Number.isSafeInteger(file.bytes) || file.bytes <= 0 || file.bytes > 32 * 1024 * 1024) {
    throw new Error(`Invalid corpus manifest entry: ${sample.id}/${file.source}`);
  }
  const directory = join(root, "target/corpus", sample.id, file.role === "input" ? "inputs" : "provenance");
  const path = join(directory, basename(file.source));
  try {
    const stat = await lstat(path);
    if (!stat.isFile() || stat.size !== file.bytes) throw new Error(`${path}: not a regular file of the expected size`);
    verify(await readFile(path), file, path);
    return;
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  const url = `https://raw.githubusercontent.com/${sample.repo}/${sample.commit}/${file.source.split("/").map(encodeURIComponent).join("/")}`;
  const response = await fetch(url, { signal: AbortSignal.timeout(60000) });
  if (!response.ok) throw new Error(`HTTP ${response.status}: ${url}`);
  const chunks = [];
  let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    if (size > file.bytes) throw new Error(`${url}: download exceeds its pinned size`);
    chunks.push(chunk);
  }
  const bytes = Buffer.concat(chunks);
  verify(bytes, file, url);
  await mkdir(directory, { recursive: true });
  const temporary = `${path}.${process.pid}.partial`;
  let created = false;
  try {
    await writeFile(temporary, bytes, { flag: "wx" });
    created = true;
    // Publishing a hard link is atomic and refuses to replace a cache file
    // created concurrently after the initial existence check.
    await link(temporary, path);
  } finally {
    if (created) await rm(temporary, { force: true });
  }
}

for (const sample of manifest.samples.filter((sample) => !requested.size || requested.has(sample.id))) {
  const names = new Set();
  for (const file of sample.files) {
    const key = `${file.role}/${basename(file.source)}`;
    if (names.has(key)) throw new Error(`${sample.id}: colliding corpus filenames: ${key}`);
    names.add(key);
  }
  // Keep network concurrency bounded, including large licence evidence documents.
  for (let i = 0; i < sample.files.length; i += 4) {
    const results = await Promise.allSettled(sample.files.slice(i, i + 4).map((file) => fetchFile(sample, file)));
    const failed = results.find((result) => result.status === "rejected");
    if (failed) throw failed.reason;
  }
  console.log(`${sample.id}: verified → target/corpus/${sample.id}/inputs`);
}
