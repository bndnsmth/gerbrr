# Gerbrr browser proof of concept

**Gerber viewing at ridiculous speed.** Safety goggles on. Gerbers in.

## Try it

**Hosted viewer:** https://gerbrr.bndnsmth.com/ — files still stay in your browser.

To run locally:

```sh
mise run web
```

Open **http://localhost:4173** in current Chrome, Edge, Firefox, or another browser
supporting WebGPU or WebGL2. Hardware acceleration is recommended. You get the
same egui workspace, not a screenshot or a separate JavaScript renderer.

- **Open board:** select a ZIP or individual Gerber/Excellon layers.
- **Folder:** select a fabrication directory on browsers supporting directory input.
- **Drag and drop:** drop a ZIP or a group of layer files onto the page.
- **Export:** download a PNG of the workspace.
- Layer appearance, isolation, measurement, grid, rotation/mirroring, and navigation
  reuse the native UI and GPU renderer.

Measurements share native snapping: press **M**, use **Auto/Centers/Edges** in the
toolbar, **N** to toggle, **Shift+N** to cycle, or hold **Alt** to place a free point.
The worker transfers shared source-shape metadata along with the mesh; the main
instance validates it and rebuilds the snap indexes. No JavaScript approximation
or server-side geometry lookup is involved. See the [measurement guide](GUIDE.md#snapped-measurements).

`window.gerbrr.measurement()` exposes read-only points, target kinds, live preview,
distance, snap settings, and camera/canvas coordinates for custom HTML displays
and browser tests. There is no measurement upload or persistence backend.

The built-in demo opens immediately on desktop. Cadence ZIPs use the same
parser/classification fixes as the native viewer. No private board files are
bundled with the web assets or served from the Downloads directory.

## Privacy and execution model

The Node server only serves static files from `web/`. It binds to `127.0.0.1`, has
no upload endpoint, and rejects HTTP methods other than GET/HEAD. Board contents
are read using the browser File API and stay in that browser session.

1. The main WASM instance runs egui + wgpu, preferring WebGPU and falling back to
   WebGL2 when no WebGPU adapter is available. Both use the same Rust renderer,
   MSAA polarity masks, and cached composites. The WebGL2 canvas retains its
   drawing buffer so PNG downloads work even when the viewer is idle.
2. File bytes are transferred to a dedicated module Web Worker.
3. A second WASM instance uses the shared Rust ZIP reader, Gerber/Excellon parser,
   classification, and tessellation code. Rayon is disabled in WASM; the browser
   worker provides the background execution instead of unsupported OS threads.
4. The worker returns a transferable binary mesh/document buffer.
5. The main instance validates/decodes that buffer, restores the inspection index,
   uploads geometry once, and uses the existing cached MSAA renderer.

Parsing doesn't block normal interaction with the previous board. Decoding the
result, rebuilding its R-tree, and initially uploading GPU buffers still happen
on the main instance, so those final steps can cause a short pause on large boards.
There are no analytics, remote fonts, authentication, or cloud storage in the app.

## Build and serve separately

```sh
mise run web-build
mise run web-serve
```

The build task installs `wasm32-unknown-unknown` through the mise-managed Rust
toolchain and the exact matching wasm-bindgen CLI in `target/tools`. It builds the
Rust library in release mode and generates `web/pkg/gerbrr.js` and its WASM
binary. These generated files are ignored by git. Node 26.8.2 and Rust 1.99.0 are
pinned in `mise.toml`; no npm packages are required for the web shell or server.

Set `PORT` to change the local server's port. It remains loopback-only. WebGPU
needs a secure context: localhost works over HTTP, other hosts normally need HTTPS.
Opening `index.html` directly as a `file://` URL is not supported. Hosting elsewhere
would require serving these static assets correctly, including the WASM MIME type.
The local development server does not deploy or expose a public server. The hosted
viewer uses Cloudflare Workers Static Assets; see [DEPLOY.md](DEPLOY.md) for the
allowlisted build, security headers, deployment commands, and CI setup.

## Verify

```sh
mise run check
cargo clippy --lib --target wasm32-unknown-unknown -- -D warnings

# Optional private-fixture smoke tests, including in-memory and worker transfer:
GERBER_TEST_ARCHIVE=/path/to/private-cadence.zip \
  cargo test -- --include-ignored
```

Tests cover in-memory ZIP/loose-file imports, folder filtering, worker result
serialization, restored inspection, corrupt payloads, native GPU compositing,
and the existing parser regressions. Browser testing additionally checks actual
WASM worker upload, the rendered WebGPU/WebGL2 canvases, and PNG export after idle.

For the optional pinned public KiCad/Altium/Cadence samples, run
`mise run corpus-check`; see [CORPUS.md](CORPUS.md). This checks browser-byte and
worker-transfer imports as well as native imports. Corpus downloads remain under
`target/corpus/`, outside the server's static root; use browser file/folder
selection to open them. It is not a substitute for testing an actual WASM canvas.

## Browser graphics troubleshooting

Some browsers expose `navigator.gpu` but return `null` from `requestAdapter()`.
Previously this caused `failed to create wgpu surface: ... {}`. Gerbrr now checks
for an actual adapter through wgpu and automatically uses WebGL2 in that case.
Browsers without the WebGPU API can use the same fallback.

For a WebGPU driver issue, explicitly select WebGL2 with
**http://localhost:4173/?renderer=webgl**. A canvas cannot switch graphics APIs
once initialized, so change the URL and reload rather than switching mid-session.
If neither backend can start, enable browser hardware acceleration, check the
browser's graphics diagnostics (`chrome://gpu` or `about:support`), and try a
current browser. No browser flags or machine-wide settings are changed by Gerbrr.

## PoC limitations

- Requires WebGPU or WebGL2; unsupported browsers display a startup explanation.
  No software CPU renderer is provided by the app. Browser-managed software GPU
  implementations may work, but can be slower than native hardware acceleration.
- Desktop-sized windows are recommended; this is not a polished mobile UI or a
  complete screen-reader/accessibility implementation.
- Small screens show a desktop-first notice with **Continue anyway**. The viewer
  does not download WASM or start the GPU until accepted or resized to a
  desktop-sized viewport. This also covers short touch-screen landscape views;
  acceptance lasts only for the current page, and resizing an existing workspace
  does not discard its board.
- Uploads use File/Directory selection, not arbitrary local filesystem access.
  Dropping a directory is browser-dependent; use **Folder** for directory selection.
- A worker is recreated for each import; there is some WASM startup/transfer cost.
- Imports time out after two minutes. Source-count/size and mesh limits remain in
  place, but actual browser/GPU memory limits can be lower.
- Boards and UI preferences are not persisted between page reloads.
- Curve approximation and CAM-sign-off limitations are the same as the native app.
- The local server is a development convenience, not production hosting software.
  Production hosts only the packaged assets, not this Node server.
