# Performance notes

Measured on October 12, 2026, with the project's Rust 1.99.0 toolchain and release
profile, on an **AMD Ryzen AI Max+ 395 / Radeon 8060S** machine.

```text
cargo bench --bench throughput

100,000 flashes: 31.75 ms · 3.15 M primitives/s · 1,400,000 triangles
Six-layer demo: 0.30 ms average
```

These are synthetic **CPU import** results, including parsing, mesh construction,
and R-tree construction. The flash test uses one warm-up and five measured
iterations; the demo uses 100 iterations. They are not GPU timings, FPS numbers,
or a comparison against other viewers. Expect different results with larger
regions, elaborate aperture macros, different hardware, or cold filesystem reads.

## Measurement snapping (October 13, 2026)

With source-shape metadata and snap indexes enabled, the same release benchmark
on this machine measured:

```text
100,000 flashes: 36.60 ms · 2.73 M primitives/s · 1,400,000 triangles
Six-layer demo: 0.35 ms average
100,000-flash center snapping: 0.75 µs/query · one shared source shape
LimeSDR Mini Auto snapping: 9.01 µs/query across visible layers
LimeSDR Mini Edges snapping: 9.08 µs/query across visible layers
```

The synthetic snap test queries 20,000 nearby pad origins. The optional LimeSDR
test uses 500 sampled top-copper flash origins, offset by 3 µm, with a 60 µm
acquisition radius, repeated 20 times. Queries returning no eligible target are
included. These are CPU query measurements, **not** total UI latency, FPS, worst-
case timings, or an independent geometry-accuracy comparison.

Repeated pads/drills share source shapes instead of allocating an edge index per
flash. Origins have a point R-tree; larger contours index groups of 32 segments.
The renderer still caches GPU composites: moving a snap marker does not rebuild
or upload the PCB mesh. `memory_bytes()` reports GPU mesh storage, not the extra
CPU source-shape/index storage used by snapping.

## Where the work goes

- Coordinates and modal commands use stack-backed token storage in the common case.
- Repeated flashes reuse tessellated aperture templates, keyed by aperture and
  transform. The previous uncached implementation measured roughly 119 ms for
  the same 100,000-flash case.
- Feature labels are shared; a million identical pad labels do not need a million
  separate string allocations.
- Independent layers import through Rayon on a background worker.
- Each imported mesh is uploaded once. Color changes, visibility, zooming, and
  panning do not retessellate or reupload geometry.
- Ordered polarity batches preserve paint semantics and allow coarse viewport
  culling without issuing a draw call for every primitive.
- Layers share one reusable MSAA mask target. The final composite is cached until
  the camera, viewport, or layer appearance changes.
- Inspection queries an R-tree instead of scanning every feature on a click.
- There is no continuous animation/repaint loop when idle.

The status bar's “ms UI” measures CPU time building the egui workspace, **not**
total frame time or GPU time. The built-in board is intentionally small; it is a
functional demo, not proof of large-board performance.

## Next performance frontiers

GPU instancing for repeated flashes would reduce mesh memory and initial upload
cost. Analytic circle/capsule shaders or multiresolution geometry would improve
extreme zoom. Fine-grained spatial GPU batching and indirect draws would help
very large boards. Those are future improvements, not features claimed by this
version.
