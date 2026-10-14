# Gerbrr user guide

The details behind the workspace: controls, measurements, supported formats, and
development commands. For a quick introduction, see the [README](README.md).

## Browser PoC

Try the hosted viewer at **https://gerbrr.bndnsmth.com/**, or run it locally:

```sh
mise run web
```

Open **http://localhost:4173** in a current browser with WebGPU or WebGL2. The same
Rust/egui workspace opens with the demo; upload/drop your Gerber ZIP to inspect a real board.
Files are parsed in a local Web Worker, **not uploaded to the server**. Layer
controls, measurement, pan/zoom, rotation, and PNG download use the shared viewer.

The first build installs the Rust WASM target and a matching wasm-bindgen CLI
inside `target/tools`; it does not install a global JS build framework. Generated
assets go in `web/pkg`. Node is pinned through mise alongside Rust.

WebGPU is preferred; the same Rust renderer falls back to WebGL2 if the browser
cannot provide a WebGPU adapter. WebGPU requires localhost or HTTPS. This PoC
targets desktop-sized displays and doesn't include a CPU renderer, hosting, or a
production browser test matrix.
See [WEB.md](WEB.md) for architecture, browser requirements, and limitations.

## Native app and `gerbrr` CLI

Rust is pinned to **1.99.0** in `mise.toml`. No separate runtime manager is needed.

```sh
mise trust
mise install rust
mise run dev
```

No arguments opens the built-in six-layer Atlas demo. Open real fabrication files
with **Open board**, drop them on the window, or pass them on the command line:

```sh
./target/release/gerbrr path/to/board.zip
./target/release/gerbrr open path/to/gerber-folder
./target/release/gerbrr inspect board.gtl board.gbl board.gto board.drl
./target/release/gerbrr screenshot board.png path/to/board.zip
./target/release/gerbrr --help
```

To put the `gerbrr` command on your Cargo bin path, optionally run
`cargo install --path . --locked`. No installation is required when using the
project-local executable or mise tasks. The original `--inspect`, `--screenshot`,
and `--write-demo` flags remain supported.

Requires a desktop session and a wgpu-compatible graphics adapter. Wayland and X11
are enabled on Linux; wgpu selects the available graphics backend. File dialogs
use the native platform integration. Board files stay local; the GitHub button
opens the project repository in your browser.

## The workspace

- **GPU-resident geometry:** parse, tessellate, and spatially index once. Pan and
  zoom change a camera uniform, not thousands of egui shapes.
- **Polarity-correct rendering:** each layer gets an isolated dark/clear mask,
  4× MSAA, and premultiplied-alpha compositing. A clear area in one layer never
  accidentally erases another layer.
- **Cached views:** unchanged artwork reuses a viewport-sized composite. No
  continuous repaint loop while idle.
- **Background import:** files parse in parallel on a worker; the existing board
  stays usable until the replacement is ready. Partial imports retain readable
  layers and report failures.
- **Layer control:** visibility, isolation, color, opacity, automatic stack
  ordering, and X2/file-name/Cadence film-metadata classification. Inner planes,
  bottom silkscreen, assembly sheets, and fabrication drawings start hidden so
  they do not obscure the top-side view. Fit follows the visible layers.
  The sidebar collapses with its top chevron; compact rows and a counted layer
  header sit above bottom-pinned properties, and the list fills the remaining height.
- **Navigation:** cursor-anchored zoom, trackpad pinch, pan, fit, quarter-turn
  rotation, back-side mirroring, an adaptive world grid, and a scale bar.
- **Tools:** snapped two-point distance/axis/angle measurement, with pad/hole
  origins, straight-slot centers, trace/slot endpoints, and source geometry edges.
  R-tree-backed feature **bounds** inspection remains a separate tool. All geometry
  and measurements use millimeters.
- **Import:** individual files, recursively scanned folders, or ZIPs read directly
  without extraction. Common Gerber and Excellon filename extensions are recognized.
- **PNG export:** save the entire workspace exactly as displayed.

The substrate is a decorative **bounds rectangle**, not a reconstructed board
outline. Disable it to inspect artwork on the bare grid.

## Shortcuts

| Action | Shortcut |
| --- | --- |
| Open files / folder | `Ctrl+O` / `Ctrl+Shift+O` |
| Fit board | `F`, `Home`, or double-click in Pan mode |
| Pan / inspect / measure | `H` / `V` / `M` |
| Toggle measurement snapping / cycle modes | `N` / `Shift+N` in Measure |
| Temporarily place a free measurement point | Hold `Alt` (`Option` on macOS) |
| Temporary pan | `Space` + drag, middle drag, or right drag |
| Zoom around cursor | Scroll, pinch, `+` / `-` |
| Rotate / mirror | `R` / `B` |
| Isolate selected layer | `S` or double-click a layer |
| Toggle grid / layer | `G` / `1`–`9` |
| Clear selection / measurement | `Esc` |
| Help | `?` |

On macOS, use Command instead of Control.

## Snapped measurements

Press **M**, hover a target, then click two points. An amber marker identifies
the current **Center**, **Endpoint**, or **Edge**; the measurement readout records
whether each endpoint was snapped or placed freely. A third click starts a new
measurement. Pan with Space + drag or middle/right drag without placing a point.

- **Auto** prefers centers, then endpoints, then nearby edges. Choose **Centers**
  (origins/endpoints) or **Edges** in the toolbar for pitch or edge-gap measurements.
  `Shift+N` cycles these modes; `N` toggles snapping; hold **Alt** for a free point.
- Acquisition uses a **10 logical-screen-point radius**, constant through zoom,
  rotation, mirroring, and display scaling. Zoom in to disambiguate dense artwork.
- Only enabled layers with nonzero opacity participate. Select a layer to break
  coincident-target ties, or isolate it to avoid other layers' targets.
- Targets use actual source contours and declared Gerber flash/NC drill origins,
  not feature-box centers, mesh diagonals, the adaptive grid, or the decorative
  substrate. Asymmetric macros may have a declared origin outside their artwork.
- Circle edges use analytic radii. Other curves follow the viewer's approximately
  **5 µm** flattening tolerance. Overlap seams and cleared-away geometry are
  excluded; very dense or ambiguous intersections may have no eligible target and
  fall back to free placement. This is not an automatic minimum-clearance solver.

Placed endpoints stay fixed when changing tools, zooming, rotating, or panning.
`Esc` clears the measurement. Review parser warnings before trusting measurements.

## Format coverage and limitations

Supported RS-274X/X2 geometry includes leading/trailing-zero and decimal
coordinates, inch/metric units, modal and incremental coordinates, circular,
rectangular, obround, and polygon apertures with circular holes; aperture
rotation/scale/mirroring; linear and single-/multi-quadrant circular interpolation;
multi-contour regions; polarity; step-repeat; and legacy image offsets.
Identity legacy image controls (`IR0`, `MIA0B0`, `SFA1B1`) are accepted; non-identity
legacy image transforms still generate warnings.

Aperture macros support variables, assignments, parentheses, arithmetic, circles,
vector lines, center/lower-left rectangles, outlines, regular polygons, moirés,
and thermals. Macro exposure is resolved with polygon Boolean operations before
tessellation, so overlapping dark primitives do not cancel each other.

Excellon supports tool definitions, decimal or declared fixed-point coordinates,
inch/metric units, zero modes, absolute/incremental positions, repeat holes, G85
slots, and linear/IJ-arc routing, including Cadence `.rou` files. `INCH,LZ` output
can use different decimal precision on each coordinate without changing scale.
Consistent, unit-matched companion Gerbers in the same archive directory can
establish an undeclared LZ integer width; explicit drill declarations always win.
Unresolved formats still generate warnings, and drill decimal precision is never
silently copied from a Gerber film.

Cadence `nc_param.txt` sidecars can explicitly declare NC precision, units, and
zero suppression. They are scoped to their own directory/archive, and inline
drill declarations win. Older Holesize-comment tool tables with `M00` tool blocks
and valid empty KiCad drill exports are supported. Non-artwork `.txt` reports are
ignored, while malformed fabrication headers still generate diagnostics.

**This is an artwork viewer, not a validated CAM sign-off tool.**

- Block apertures (`AB`), old image inversion/axis transforms, knockout, embedded
  include files, and drawing with compound/non-convex apertures are not supported.
  Review import diagnostics before trusting a rendering.
- Unsupported macro primitives fail that layer; unsupported parameters are
  reported. X2 metadata classifies layers but is not an electrical net model.
- Inspection selects feature bounds, not exact copper coverage or connectivity.
  Measurement snapping guides point placement; it does not infer electrical
  connectivity, run manufacturing DRC, or certify minimum copper clearance.
- Curves are tessellated at approximately **5 μm** tolerance; macro circles use
  polygon approximations. Extreme magnification can reveal those approximations.
- Imports are bounded to 256 layers, 128 MiB per source, and 512 MiB total source
  data, with additional primitive/mesh/repeat limits. ZIPs are never unpacked.

## Verify and benchmark

```sh
mise run check
cargo bench --bench throughput
```

Tests cover coordinate formats, aperture holes, polarity, regions, macro
arithmetic/Boolean geometry, interpolation, transforms, repetition, drill routing,
partial imports, the demo, and camera transforms. A headless GPU readback test
checks real layer compositing and cached-view invalidation; it skips if no graphics
adapter is available.

Cadence regressions use synthetic exports rather than bundling private board data.
To also exercise the optional local Cadence regression fixture:

```sh
GERBER_TEST_ARCHIVE=/path/to/private-cadence.zip \
  cargo test --test cadence -- --include-ignored
```

An optional [public regression corpus](CORPUS.md) adds eight real KiCad, Altium,
and Cadence designs, pinned by commit and SHA-256. Downloads stay under `target/`
and are not bundled into the viewer:

```sh
mise run corpus-check
```

The benchmark measures synthetic **CPU import** (parser + tessellation + spatial
index), not GPU frame rate. It includes 100,000 flashes and the six-layer demo.
“Fastest in the world” is an ambition, not an unverified comparative claim.
See [PERFORMANCE.md](PERFORMANCE.md) for measured results and the rendering strategy.

## Useful commands

```sh
# Validate imports without opening a window; warnings produce a nonzero exit.
./target/release/gerbrr inspect path/to/board.zip

# Write a fresh set of demo sources, without overwriting existing files.
# A ready-to-open set is already included in examples/atlas.
./target/release/gerbrr demo-export path/to/new-folder

# Capture a workspace PNG and exit; a desktop session is required.
./target/release/gerbrr screenshot board.png path/to/board.zip
```

## Architecture

`src/parser.rs` interprets fabrication commands; `src/geometry.rs` tessellates
paths with lyon and builds an R-tree; `src/document.rs` imports files/ZIPs and
constructs the demo. `src/renderer.rs` owns persistent GPU buffers and cached
MSAA/composite targets. `src/app.rs` is the egui workspace. CPU parser tests sample
the generated mesh, while the GPU test reads back the actual rendered pixels.

MIT licensed. Dependencies retain their respective licenses.
