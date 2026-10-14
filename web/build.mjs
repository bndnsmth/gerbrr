import { spawnSync } from "node:child_process";
import { mkdirSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { buildSite } from "../scripts/build-site.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
if (process.argv.slice(2).some((arg) => arg !== "--production")) throw new Error("Usage: node web/build.mjs [--production]");
const version = "0.2.129"; // Must match wasm-bindgen in Cargo.toml / Cargo.lock.
const executable = join(root, "target/tools/bin", process.platform === "win32" ? "wasm-bindgen.exe" : "wasm-bindgen");
function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
run("rustup", ["target", "add", "wasm32-unknown-unknown"]);
const installed = existsSync(executable) && spawnSync(executable, ["--version"], { encoding: "utf8" }).stdout?.trim() === `wasm-bindgen ${version}`;
if (!installed) run("cargo", ["install", "wasm-bindgen-cli", "--version", version, "--locked", "--root", "target/tools"]);
run("cargo", ["build", "--locked", "--release", "--lib", "--target", "wasm32-unknown-unknown"]);
mkdirSync(join(root, "web/pkg"), { recursive: true });
run(executable, ["target/wasm32-unknown-unknown/release/gerbrr.wasm", "--target", "web", "--out-dir", "web/pkg", "--out-name", "gerbrr", "--no-typescript"]);
if (process.argv.includes("--production")) {
  const build = await buildSite({ root });
  console.log(`Gerbrr production build ready in dist/ · release ${build.release}`);
} else {
  console.log("Gerbrr browser build ready in web/pkg. Run mise run web-serve to view it.");
}
