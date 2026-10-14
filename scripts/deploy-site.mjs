// Wrangler's non-interactive custom-domain deploy can overwrite DNS conflicts.
// Deploy assets without routes, then bind this one hostname with overrides OFF.
import { spawnSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { loadEnvFile } from "node:process";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const project = fileURLToPath(new URL("../", import.meta.url));
export const hostname = "gerbrr.bndnsmth.com";
const zoneName = "bndnsmth.com";

export function deploymentAccountId(env = process.env) {
  const value = env.CLOUDFLARE_ACCOUNT_ID;
  if (typeof value !== "string" || !/^[a-f0-9]{32}$/i.test(value.trim())) {
    throw new Error("Set a valid CLOUDFLARE_ACCOUNT_ID in your environment or ignored .env.local");
  }
  return value.trim().toLowerCase();
}

export function verifyZone(zones, accountId) {
  if (!accountId || !Array.isArray(zones) || zones.length !== 1 || zones[0].name !== zoneName
      || zones[0].status !== "active" || zones[0].account?.id !== accountId) {
    throw new Error(`Expected the active ${zoneName} zone in the configured Cloudflare account`);
  }
  return zones[0].id;
}

export function verifyDomains(domains, worker) {
  if (!Array.isArray(domains)) throw new Error("Invalid Cloudflare domain inventory");
  const matches = domains.filter(domain => domain.hostname === hostname);
  if (matches.some(domain => domain.service !== worker)) {
    throw new Error(`${hostname} belongs to another Worker; refusing to replace it`);
  }
  return matches;
}

export function verifyChangeset(changeset, existing) {
  if (!["added", "updated", "removed", "conflicting"].every(key => Array.isArray(changeset?.[key]))) {
    throw new Error("Invalid Cloudflare domain changeset; refusing to change DNS");
  }
  if (changeset.conflicting.length || changeset.removed.length) {
    throw new Error("Existing DNS records or domain removals need approval; refusing to overwrite them");
  }
  if ([...changeset.added, ...changeset.updated].some(domain => domain.hostname !== hostname)) {
    throw new Error("Domain changeset includes an unrelated hostname");
  }
  const owned = new Set(existing.map(domain => domain.id));
  if (changeset.updated.some(domain => !owned.has(domain.id))) {
    throw new Error("Domain changeset updates an unowned binding");
  }
}

function wrangler(args, { quiet = false, accountId } = {}) {
  const result = spawnSync(process.execPath, [join(project, "node_modules/wrangler/bin/wrangler.js"), ...args], {
    cwd: project, stdio: quiet ? "pipe" : "inherit",
    env: { ...process.env, ...(accountId ? { CLOUDFLARE_ACCOUNT_ID: accountId } : {}), WRANGLER_SEND_METRICS: "false" },
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    if (quiet) { process.stderr.write(result.stdout ?? ""); process.stderr.write(result.stderr ?? ""); }
    throw new Error(`Wrangler exited with status ${result.status}`);
  }
}

async function token() {
  if (process.env.CLOUDFLARE_API_TOKEN) return process.env.CLOUDFLARE_API_TOKEN;
  // Let Wrangler refresh an existing local login, without creating a new login.
  wrangler(["whoami"], { quiet: true });
  const paths = [
    join(process.env.XDG_CONFIG_HOME ?? join(homedir(), ".config"), ".wrangler/config/default.toml"),
    join(homedir(), ".wrangler/config/default.toml"),
  ];
  for (const path of paths) {
    try {
      const value = (await readFile(path, "utf8")).match(/^oauth_token\s*=\s*"([^"]+)"/m)?.[1];
      if (value) return value;
    } catch (error) { if (error.code !== "ENOENT") throw error; }
  }
  throw new Error("Run npm exec -- wrangler login, or set a scoped CLOUDFLARE_API_TOKEN");
}

export async function deploy({ preview = false } = {}) {
  // External environment variables take precedence over this untracked local file.
  try { loadEnvFile(join(project, ".env.local")); }
  catch (error) { if (error.code !== "ENOENT") throw error; }
  const account = deploymentAccountId();
  const config = JSON.parse(await readFile(join(project, "wrangler.jsonc"), "utf8"));
  if (Object.hasOwn(config, "account_id") || Object.values(config.env ?? {}).some(env => Object.hasOwn(env, "account_id"))) {
    throw new Error("Account IDs belong in environment settings, not Wrangler configuration");
  }
  if (config.routes?.length || config.env.production.routes?.length) {
    throw new Error("Custom domains must be bound by the guarded deploy script, not Wrangler routes");
  }
  const worker = preview ? config.name : config.env.production.name;
  if (worker !== (preview ? "gerbrr-preview" : "gerbrr")) throw new Error("Unexpected deployment target");
  const auth = await token();
  async function api(path, method = "GET", body) {
    const response = await fetch(`https://api.cloudflare.com/client/v4/${path}`, {
      method, headers: { Authorization: `Bearer ${auth}`, "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(30000),
    });
    const data = await response.json();
    if (!response.ok || !data.success) throw new Error(`Cloudflare request failed: ${JSON.stringify(data.errors)}`);
    return data.result;
  }
  const zone = verifyZone(await api(`zones?name=${zoneName}&account.id=${account}`), account);
  const existing = verifyDomains(await api(`accounts/${account}/workers/domains`), config.env.production.name);
  const origins = [{ hostname, zone_id: zone, previews_enabled: false }];
  const path = `accounts/${account}/workers/scripts/${worker}/domains`;
  // Existing production deployments can be checked before any new assets upload.
  if (!preview && existing.length) {
    verifyChangeset(await api(`${path}/changeset?replace_state=false`, "POST", origins), existing);
  }
  wrangler(["deploy", "--env", preview ? "" : "production"], { accountId: account });
  if (!preview) {
    verifyChangeset(await api(`${path}/changeset?replace_state=false`, "POST", origins), existing);
    await api(`${path}/records`, "PUT", {
      override_scope: false, override_existing_origin: false, override_existing_dns_record: false, origins,
    });
    console.log(`Production custom domain bound without DNS overrides: https://${hostname}/`);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  if (args.some(arg => arg !== "--preview")) throw new Error("Usage: node scripts/deploy-site.mjs [--preview]");
  await deploy({ preview: args.includes("--preview") });
}
