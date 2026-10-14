import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { buildSite } from "../scripts/build-site.mjs";
import { deploymentAccountId, hostname, verifyChangeset, verifyDomains, verifyZone } from "../scripts/deploy-site.mjs";

const project = fileURLToPath(new URL("../", import.meta.url));
async function fixture(t) {
  await mkdir(join(project, "target"), { recursive: true });
  const root = await mkdtemp(join(project, "target/site-test-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const files = {
    "web/index.html": '<link href="./style.css"><link href="./icon.svg"><img src="./icon.svg"><script type="module" src="./main.js"></script>',
    "web/main.js": 'new Worker(new URL("./worker.js", import.meta.url), { type: "module" }); import("./pkg/gerbrr.js");',
    "web/worker.js": 'import init from "./pkg/gerbrr.js";',
    "web/style.css": "body { color: white; }",
    "web/icon.svg": '<svg xmlns="http://www.w3.org/2000/svg"/>',
    "web/pkg/gerbrr.js": 'new URL("gerbrr_bg.wasm", import.meta.url)',
    "web/pkg/gerbrr_bg.wasm": Buffer.from([0, 97, 115, 109, 1, 0, 0, 0]),
    "web/_headers": "/*\n  X-Content-Type-Options: nosniff\n",
    "web/robots.txt": "User-agent: *\nAllow: /\n",
    "assets/preview.png": "original demo screenshot",
    "LICENSE": "MIT",
    "assets/octicons-LICENSE.txt": "Octicons MIT",
    "web/serve.mjs": "PRIVATE DEVELOPMENT SCRIPT",
    "web/pkg/unexpected.txt": "DO NOT PUBLISH",
    "target/corpus/secret.gbr": "PRIVATE FABRICATION FILE",
    ".env": "PRIVATE CREDENTIAL",
    ".env.local": "PRIVATE DEPLOYMENT SETTINGS",
  };
  for (const [path, content] of Object.entries(files)) {
    await mkdir(join(root, path, ".."), { recursive: true });
    await writeFile(join(root, path), content);
  }
  return root;
}

async function walk(root, prefix = "") {
  const result = [];
  for (const entry of await readdir(join(root, prefix), { withFileTypes: true })) {
    const path = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) result.push(...await walk(root, path)); else result.push(path);
  }
  return result.sort();
}

test("production output only contains explicit public assets and keeps module paths together", async (t) => {
  const root = await fixture(t);
  const build = await buildSite({ root });
  const expected = [...build.files, ".gerbrr-site.json"].sort();
  assert.deepEqual(await walk(join(root, "dist")), expected);
  const html = await readFile(join(root, "dist/index.html"), "utf8");
  assert.ok(html.includes(`/assets/${build.release}/main.js`));
  assert.ok(html.includes(`/assets/${build.release}/style.css`));
  assert.equal((html.match(new RegExp(`/assets/${build.release}/icon.svg`, "g")) ?? []).length, 2);
  assert.equal(await readFile(join(root, `dist/assets/${build.release}/worker.js`), "utf8"), 'import init from "./pkg/gerbrr.js";');
  assert.equal(await readFile(join(root, "dist/.assetsignore"), "utf8"), ".gerbrr-site.json\n");
  for (const file of build.files) {
    const content = await readFile(join(root, "dist", file), "utf8");
    assert.ok(!content.includes("PRIVATE"));
    assert.ok(!content.includes("DO NOT PUBLISH"));
  }
});

test("build IDs are deterministic and change with either parser bundle or worker", async (t) => {
  const root = await fixture(t);
  const first = await buildSite({ root });
  assert.equal((await buildSite({ root })).release, first.release);
  await writeFile(join(root, "web/worker.js"), "changed worker");
  const second = await buildSite({ root });
  assert.notEqual(second.release, first.release);
  assert.ok(!(await walk(join(root, "dist"))).some(p => p.includes(first.release)));
  await writeFile(join(root, "web/pkg/gerbrr.js"), "changed WASM loader");
  assert.notEqual((await buildSite({ root })).release, second.release);
});

test("missing or invalid browser artifacts leave the previous build untouched", async (t) => {
  const root = await fixture(t);
  const first = await buildSite({ root });
  await writeFile(join(root, "web/pkg/gerbrr_bg.wasm"), "not WASM");
  await assert.rejects(buildSite({ root }), /valid WASM/);
  const saved = JSON.parse(await readFile(join(root, "dist/.gerbrr-site.json"), "utf8"));
  assert.equal(saved.release, first.release);
  await rm(join(root, "web/worker.js"));
  await assert.rejects(buildSite({ root }), /ENOENT/);
  assert.equal(JSON.parse(await readFile(join(root, "dist/.gerbrr-site.json"), "utf8")).release, first.release);
});

test("an unrelated dist directory or symlink is never replaced", async (t) => {
  const root = await fixture(t);
  await mkdir(join(root, "dist"));
  await writeFile(join(root, "dist/user-work.txt"), "preserve me");
  await assert.rejects(buildSite({ root }), /unrecognized/);
  assert.equal(await readFile(join(root, "dist/user-work.txt"), "utf8"), "preserve me");
  await rm(join(root, "dist"), { recursive: true });
  await symlink(join(root, "assets"), join(root, "dist"));
  await assert.rejects(buildSite({ root }), /symlink/);
});

test("symlinked public assets, oversized assets, and missing HTML references are rejected", async (t) => {
  const root = await fixture(t);
  await rm(join(root, "web/worker.js"));
  await symlink(join(root, ".env"), join(root, "web/worker.js"));
  await assert.rejects(buildSite({ root }), /regular public asset/);
  await rm(join(root, "web/worker.js"));
  await writeFile(join(root, "web/worker.js"), Buffer.alloc(25 * 1024 * 1024 + 1));
  await assert.rejects(buildSite({ root }), /25 MiB/);
  await writeFile(join(root, "web/worker.js"), "worker");
  await writeFile(join(root, "web/index.html"), "unexpected shell");
  await assert.rejects(buildSite({ root }), /shell reference/);
});

test("production CSP supports local WASM and workers without allowing remote scripts", async () => {
  const headers = await readFile(join(project, "web/_headers"), "utf8");
  assert.match(headers, /script-src 'self' 'wasm-unsafe-eval'/);
  assert.match(headers, /worker-src 'self'/);
  assert.match(headers, /connect-src 'self'/);
  assert.match(headers, /frame-ancestors 'none'/);
  assert.match(headers, /Cache-Control: public, max-age=31536000, immutable/);
  assert.match(headers, /Cache-Control: public, max-age=0, must-revalidate, no-transform/);
  assert.match(headers, /X-Robots-Tag: noindex/);
  assert.ok(!headers.includes("script-src 'unsafe-inline'"));
});

test("deployment checks reject another account, zone, or Worker binding", () => {
  const account = "a".repeat(32); // Synthetic account, never a real deployment ID.
  const zone = { id: "zone", name: "bndnsmth.com", status: "active", account: { id: account } };
  assert.equal(verifyZone([zone], account), "zone");
  for (const zones of [[], [zone, zone], [{ ...zone, status: "pending" }], [{ ...zone, account: { id: "wrong" } }]]) {
    assert.throws(() => verifyZone(zones, account), /configured Cloudflare account/);
  }
  assert.throws(() => verifyZone([zone]), /configured Cloudflare account/);
  assert.deepEqual(verifyDomains([{ hostname: "other.bndnsmth.com", service: "other" }], "gerbrr"), []);
  assert.throws(() => verifyDomains([{ hostname, service: "someone-else" }], "gerbrr"), /another Worker/);
});

test("account IDs are required at deployment time and absent from Wrangler configuration", async () => {
  const account = "a".repeat(32);
  assert.equal(deploymentAccountId({ CLOUDFLARE_ACCOUNT_ID: account }), account);
  assert.equal(deploymentAccountId({ CLOUDFLARE_ACCOUNT_ID: ` ${account}\n` }), account);
  assert.equal(deploymentAccountId({ CLOUDFLARE_ACCOUNT_ID: account.toUpperCase() }), account);
  for (const value of [undefined, "", "wrong", "../other-account", 123, `${account}&name=other`]) {
    assert.throws(() => deploymentAccountId({ CLOUDFLARE_ACCOUNT_ID: value }), /CLOUDFLARE_ACCOUNT_ID/);
  }
  const config = JSON.parse(await readFile(join(project, "wrangler.jsonc"), "utf8"));
  assert.equal(Object.hasOwn(config, "account_id"), false);
  assert.ok(Object.values(config.env ?? {}).every(env => !Object.hasOwn(env, "account_id")));
});

test("DNS conflicts, removals, unrelated hostnames, and unowned updates always fail closed", () => {
  const clean = { added: [{ hostname }], updated: [], removed: [], conflicting: [] };
  verifyChangeset(clean, []);
  verifyChangeset({ ...clean, added: [], updated: [{ id: "ours", hostname }] }, [{ id: "ours" }]);
  for (const change of [null, { ...clean, conflicting: [{ hostname }] }, { ...clean, removed: [{ hostname }] },
    { ...clean, added: [{ hostname: "other.bndnsmth.com" }] }, { ...clean, updated: [{ id: "not-ours", hostname }] }]) {
    assert.throws(() => verifyChangeset(change, []));
  }
});
