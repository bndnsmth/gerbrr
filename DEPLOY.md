# Website deployment

**Production:** https://gerbrr.bndnsmth.com/

Hosted as **Cloudflare Workers Static Assets**. There is
no server-side parser, database, upload endpoint, or application analytics.
Cloudflare still handles ordinary CDN traffic and infrastructure logs.

## Build and verify

```sh
mise trust
mise install
npm ci --ignore-scripts
mise run check
npm test
npm run build
npm exec -- playwright install chromium
npm run test:browser
```

The browser smoke test starts a loopback-only Wrangler preview on a free port.
It exercises actual WebGPU and WebGL2 rendering, worker imports, snapped center
pitch/edge gaps, idle PNG readback, CSP, WASM MIME types, caching, and missing-asset
404s. It fails on remote script requests or board uploads. Use
`GERBRR_BROWSER_EXECUTABLE=/usr/bin/chromium` to use an existing browser instead.
It also checks the small-screen notice at three phone sizes and in touch-screen
landscape, deferred WASM loading, the explicit opt-in, and resize behavior.

For interactive testing, run `npm run preview` and open http://localhost:4174.

## Deploy from this machine

An existing Wrangler login can be used, or authenticate with
`npm exec -- wrangler login`. CI uses a separate, scoped API token.

Copy `.env.example` to `.env.local` and set `CLOUDFLARE_ACCOUNT_ID` from your own
Cloudflare dashboard. This file is ignored by Git and is never packaged into the
website. You can instead set environment variables directly; they take precedence
over the local file. A local API token is optional if Wrangler is already logged in.

```sh
# Rebuild and deploy the public preview without touching production DNS:
mise run site-deploy-preview

# Rebuild and deploy production, refusing DNS conflicts:
mise run site-deploy

# Test the deployed site:
node scripts/smoke-site.mjs https://gerbrr.bndnsmth.com/
```

The guarded deploy script verifies the account, active zone, domain ownership,
and Cloudflare's proposed changes. It creates/updates only the Gerbrr custom
domain, with DNS/origin overrides explicitly disabled. Existing conflicting
records require a separate decision; other subdomains are left alone.

**Use these tasks rather than a bare production `wrangler deploy`.** The Wrangler
configuration intentionally has no custom-domain routes: unattended Wrangler
route deployment can overwrite conflicts. `scripts/deploy-site.mjs` binds the
hostname safely after the static assets have deployed. Cloudflare provisions TLS.
First-time DNS/certificate propagation can take a few minutes.

## What gets published

`scripts/build-site.mjs` packages an explicit allowlist into generated `dist/`:

- The HTML shell, CSS, JavaScript, parser worker, mascot, and compiled WASM.
- The original Atlas demo screenshot, robots file, and license notices.
- Security/caching rules in `_headers` (interpreted by Cloudflare, not served).

Build scripts, source maps, credentials, public regression downloads, and private
fabrication files are never copied. Unknown existing `dist/` directories and
symlinked input files are rejected rather than overwritten or followed.

All runtime modules share a content-fingerprinted release directory. Those assets
are immutable-cached; HTML is revalidated. Open tabs may need a refresh after a
new deployment, but never load a new parser into an old worker bundle. Missing
assets return 404, not HTML. No service worker or offline persistence is installed.

`Cache-Control: no-transform` prevents Cloudflare from automatically injecting its
zone-wide analytics beacon into this site. CSP also refuses remote scripts. This
does not change analytics settings for other hosts. Preview responses are marked
`noindex`; the production URL is canonical.

## GitHub automation

`.github/workflows/site.yml` is prepared for `bndnsmth/gerbrr`:

1. Pull requests run native/WASM checks, packaging/DNS safety tests, and both
   renderer smoke tests. They cannot deploy or access Cloudflare secrets.
2. Successful `main` pushes upload the tested production artifact and deploy that
   exact artifact. A manual run on `main` follows the same checks.
3. Production deployments are serialized; a newer build does not cancel a deploy
   already in progress. Third-party actions and npm tools are pinned.

To activate it, configure the repository's **production** environment with
`CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN` as secrets. Use Cloudflare's
**Edit Cloudflare Workers** token template, scoped to the hosting account and the
production domain's zone. Add approval and `main`-only deployment restrictions
to the environment if desired. Account-specific settings stay out of source control.
Do not copy the broad local OAuth login into GitHub secrets.

## Rollback

In **Cloudflare → Workers & Pages → gerbrr → Deployments**, roll back to a
previous successful deployment. Worker static assets are versioned with the
deployment, so the corresponding shell and WASM roll back together. Leave the
custom domain attached. Refresh the browser afterward.
