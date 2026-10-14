// Exercise the actual shipped WASM, worker, CSP, and canvases—not mock geometry.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { createServer } from "node:net";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const root = fileURLToPath(new URL("../", import.meta.url));
let server;
let browser;
let log = "";

async function localServer() {
  const reservation = createServer();
  reservation.listen(0, "127.0.0.1");
  await once(reservation, "listening");
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  server = spawn(process.execPath, [join(root, "node_modules/wrangler/bin/wrangler.js"),
    "dev", "--local", "--ip", "127.0.0.1", "--port", String(port)], {
    cwd: root, stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, WRANGLER_SEND_METRICS: "false" },
  });
  server.stdout.on("data", data => { log += data; });
  server.stderr.on("data", data => { log += data; });
  const url = `http://127.0.0.1:${port}/`;
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    if (server.exitCode !== null) throw new Error(`Local preview exited:\n${log}`);
    try { if ((await fetch(url, { signal: AbortSignal.timeout(1000) })).status === 200) return url; } catch {}
    await new Promise(resolve => setTimeout(resolve, 200));
  }
  throw new Error(`Local production preview did not become ready:\n${log}`);
}

async function clickWorld(page, point, dx = 0, dy = 0, alt = false) {
  const p = await page.evaluate(({ point, dx, dy }) => {
    const s = window.gerbrr.measurement();
    const r = s.rotation * Math.PI / 180;
    const x = (point[0] - s.center[0]) * (s.mirrored ? -1 : 1);
    const y = point[1] - s.center[1];
    const [left, top, width, height] = s.canvas;
    return { x: left + width / 2 + (Math.cos(r) * x - Math.sin(r) * y) * s.scale + dx,
      y: top + height / 2 - (Math.sin(r) * x + Math.cos(r) * y) * s.scale + dy };
  }, { point, dx, dy });
  if (alt) await page.keyboard.down("Alt");
  try { await page.mouse.click(p.x, p.y); } finally { if (alt) await page.keyboard.up("Alt"); }
  await page.waitForTimeout(100);
}

async function waitReady(page) {
  await page.waitForFunction(() => ["true", "error"].includes(document.body.dataset.ready), null, { timeout: 60000 });
  const state = await page.evaluate(() => ({ ready: document.body.dataset.ready,
    status: document.querySelector("#startup-status").textContent }));
  assert.equal(state.ready, "true", state.status);
}

async function smallScreenSmoke(base) {
  const context = await browser.newContext({
    viewport: { width: 430, height: 932 }, isMobile: true, hasTouch: true, deviceScaleFactor: 2,
  });
  const page = await context.newPage();
  const requests = [];
  const errors = [];
  page.on("request", request => requests.push(request.url()));
  page.on("pageerror", error => errors.push(error.message));
  const compiledRequests = () => requests.filter(url => url.includes("/pkg/") || url.endsWith(".wasm"));
  const prompt = page.getByRole("dialog", { name: "Better on a bigger screen." });
  try {
    await page.goto(base);
    await prompt.waitFor({ state: "visible" });
    assert.equal(await page.evaluate(() => window.matchMedia("(pointer: coarse)").matches), true);
    assert.ok((await prompt.textContent()).includes("Nothing is uploaded."));
    for (const viewport of [{ width: 320, height: 568 }, { width: 390, height: 844 },
      { width: 430, height: 932 }, { width: 932, height: 430 }]) {
      await page.setViewportSize(viewport);
      await prompt.waitFor({ state: "visible" });
      const bounds = await prompt.evaluate(dialog => ({
        horizontalOverflow: dialog.scrollWidth > dialog.clientWidth,
        card: document.querySelector(".screen-card").getBoundingClientRect().toJSON(),
        width: window.innerWidth,
        buttonHeight: document.querySelector("#continue-small-screen").getBoundingClientRect().height,
        buttonBottom: document.querySelector("#continue-small-screen").getBoundingClientRect().bottom,
        height: window.innerHeight,
      }));
      assert.equal(bounds.horizontalOverflow, false);
      assert.ok(bounds.card.left >= 0 && bounds.card.right <= bounds.width);
      assert.ok(bounds.buttonHeight >= 48);
      assert.ok(bounds.buttonBottom <= bounds.height, "Continue should be visible without scrolling");
    }
    await page.keyboard.press("Escape");
    assert.equal(await prompt.isVisible(), true, "Escape must not silently bypass the explicit choice");
    assert.deepEqual(compiledRequests(), [], "Small screens should not download the WASM before opting in");
    assert.equal(await page.evaluate(() => typeof window.gerbrr?.summary === "function"), false);

    await page.getByRole("button", { name: "Continue anyway" }).tap();
    await waitReady(page);
    assert.equal(await prompt.isVisible(), false);
    assert.ok(compiledRequests().some(url => url.endsWith(".wasm")));
    assert.equal(await page.evaluate(() => window.gerbrr.summary().layers), 6);
    await page.setViewportSize({ width: 390, height: 844 });
    assert.equal(await prompt.isVisible(), false, "Do not nag again after acceptance in the same page");

    const oldCount = compiledRequests().length;
    await page.reload();
    await prompt.waitFor({ state: "visible" });
    assert.equal(compiledRequests().length, oldCount, "Reload should return to the notice without loading WASM");
    await page.setViewportSize({ width: 1024, height: 768 });
    await waitReady(page);
    assert.equal(await prompt.isVisible(), false, "A desktop-sized viewport should start without a click");
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ smallScreen: true, phoneSizes: 3, touchLandscape: true,
      deferredWasm: true, continueAnyway: true, perPageChoice: true, desktopResize: true }));
  } finally { await context.close(); }
}

async function smoke(base, renderer) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 960 } });
  const page = await context.newPage();
  const errors = [];
  const requests = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("request", request => requests.push({ url: request.url(), method: request.method() }));
  page.on("console", message => {
    if (message.type() === "error" && /Content Security Policy|Refused to|violates.*directive/i.test(message.text())) errors.push(message.text());
  });
  const url = new URL(base);
  if (renderer === "webgl") url.searchParams.set("renderer", "webgl");
  try {
    const response = await page.goto(url.href);
    assert.equal(response.status(), 200);
    const headers = await response.allHeaders();
    assert.match(headers["content-security-policy"] ?? "", /wasm-unsafe-eval/);
    assert.equal(headers["x-content-type-options"], "nosniff");
    assert.ok(!(await response.text()).includes("static.cloudflareinsights.com"), "Cloudflare injected an analytics script");
    await waitReady(page);
    assert.equal(await page.locator("#small-screen").isVisible(), false, "Desktop must not show a mobile notice");
    const gpu = await page.evaluate(() => window.gerbrr.summary().gpu);
    if (renderer === "webgpu") assert.match(gpu, /BrowserWebGpu/, "WebGPU run silently fell back");
    else assert.ok(!gpu.includes("BrowserWebGpu"), "Forced WebGL used WebGPU");

    const source = "%FSLAX36Y36*%%MOMM*%%TF.FileFunction,Copper,L1,Top*%%ADD10C,1*%D10*X0Y0D03*X2540000Y0D03*M02*";
    await page.locator("#file-input").setInputFiles({ name: "smoke-test.gtl", mimeType: "text/plain", buffer: Buffer.from(source) });
    await page.waitForFunction(() => document.body.dataset.importing === "false" && window.gerbrr.summary().layers === 1, null, { timeout: 60000 });
    assert.equal(await page.evaluate(() => window.gerbrr.summary().primitives), 2);
    // Wait for the imported board's fit, not just its queued document update.
    await page.waitForFunction(() => {
      const state = window.gerbrr.measurement();
      return Math.abs(state.center[0] - 1.27) < 0.001 && state.scale > 100;
    });
    await page.mouse.move(700, 500);
    await page.keyboard.press("m");
    await page.waitForFunction(() => window.gerbrr.measurement().preview !== null);
    await clickWorld(page, [0, 0], 6, 2);
    await page.waitForFunction(() => window.gerbrr.measurement().points.length === 1);
    await clickWorld(page, [2.54, 0], -6, 2);
    await page.waitForFunction(() => window.gerbrr.measurement().points.length === 2);
    let measurement = await page.evaluate(() => window.gerbrr.measurement());
    assert.ok(measurement.points.every(point => point.kind === "Center"));
    assert.ok(Math.abs(measurement.distance - 2.54) < 0.00001, JSON.stringify(measurement));
    // Keep Shift held until egui has consumed each mode change; zero-duration
    // synthetic key chords otherwise release the modifier in the same frame.
    await page.keyboard.down("Shift");
    try {
      await page.keyboard.press("n");
      await page.waitForFunction(() => window.gerbrr.measurement().mode === "Centers");
      await page.keyboard.press("n");
      await page.waitForFunction(() => window.gerbrr.measurement().mode === "Edges");
    } finally { await page.keyboard.up("Shift"); }
    await page.keyboard.press("m");
    await page.waitForFunction(() => window.gerbrr.measurement().points.length === 0);
    await clickWorld(page, [0.5, 0], -3);
    await clickWorld(page, [2.04, 0], 3);
    measurement = await page.evaluate(() => window.gerbrr.measurement());
    assert.ok(measurement.points.every(point => point.kind === "Edge"));
    assert.ok(Math.abs(measurement.distance - 1.54) < 0.0001);

    // Validate the idle export path with PNG -> ImageBitmap -> 2D readback.
    await page.mouse.move(600, 100);
    await page.waitForTimeout(300);
    const png = await page.evaluate(async () => {
      const canvas = document.querySelector("#gerbrr");
      const blob = await new Promise(resolve => canvas.toBlob(resolve, "image/png"));
      if (!blob) throw new Error("Canvas PNG export failed");
      const image = await createImageBitmap(blob);
      const copy = document.createElement("canvas");
      copy.width = image.width; copy.height = image.height;
      const ctx = copy.getContext("2d"); ctx.drawImage(image, 0, 0);
      const pixels = ctx.getImageData(0, 0, copy.width, copy.height).data;
      let copper = 0;
      for (let i = 0; i < pixels.length; i += 4) if (pixels[i] > 180 && pixels[i + 1] > 80 && pixels[i + 1] < 210 && pixels[i + 2] < 150) copper++;
      image.close();
      return { bytes: blob.size, copperPixels: copper };
    });
    assert.ok(png.bytes > 1000 && png.copperPixels > 1000, "PNG does not contain the rendered pads");
    assert.equal(await page.evaluate(() => window.gerbrr.panic_message() ?? null), null);
    assert.deepEqual(errors, []);
    assert.ok(requests.every(request => request.method === "GET" && new URL(request.url).origin === url.origin),
      `Unexpected uploads or remote requests: ${JSON.stringify(requests)}`);
    const wasm = requests.find(request => request.url.endsWith(".wasm"));
    assert.ok(wasm && /\/assets\/[a-f0-9]{20}\/pkg\//.test(wasm.url));
    const wasmResponse = await context.request.get(wasm.url);
    assert.match(wasmResponse.headers()["content-type"], /application\/wasm/);
    assert.match(wasmResponse.headers()["cache-control"] ?? "", /immutable/);
    const missing = await context.request.get(new URL("assets/not-a-release/missing.wasm", base).href);
    assert.equal(missing.status(), 404, "Missing WASM incorrectly received the app shell");
    // Resizing an existing desktop workspace must not restart it or lose a board.
    await page.evaluate(() => { window.beforeScreenNotice = window.gerbrr; });
    const wasmLoads = requests.filter(request => request.url.endsWith(".wasm")).length;
    await page.setViewportSize({ width: 899, height: 960 });
    await page.locator("#small-screen").waitFor({ state: "visible" });
    await page.setViewportSize({ width: 900, height: 960 });
    await page.locator("#small-screen").waitFor({ state: "hidden" });
    assert.equal(await page.evaluate(() => window.gerbrr === window.beforeScreenNotice), true);
    assert.equal(await page.evaluate(() => window.gerbrr.summary().primitives), 2);
    assert.equal(requests.filter(request => request.url.endsWith(".wasm")).length, wasmLoads);
    console.log(JSON.stringify({ renderer, gpu, centerPitch: 2.54, edgeGap: 1.54, png, uploads: 0, cspErrors: 0 }));
  } finally { await context.close(); }
}

try {
  if (process.argv.length > 3) throw new Error("Usage: node scripts/smoke-site.mjs [https://site/]");
  const base = process.argv[2] ?? await localServer();
  browser = await chromium.launch({
    headless: true, executablePath: process.env.GERBRR_BROWSER_EXECUTABLE || undefined,
    args: ["--enable-unsafe-webgpu", "--use-angle=swiftshader", "--enable-features=Vulkan", "--use-vulkan=swiftshader", "--disable-vulkan-surface"],
  });
  await smallScreenSmoke(base);
  await smoke(base, "webgpu");
  await smoke(base, "webgl");
} finally {
  if (browser) await browser.close();
  if (server && server.exitCode === null) {
    server.kill("SIGTERM");
    await Promise.race([once(server, "exit"), new Promise(resolve => setTimeout(resolve, 5000))]);
    if (server.exitCode === null) server.kill("SIGKILL");
  }
}
