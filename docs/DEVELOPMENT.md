# Implementation status

The README is the target specification. This first increment implements the
**milestone 0 Vulkan foundation**, with the coordinate and camera building blocks
for milestone 1. It is not yet the complete Stage 1 matter browser.

## Run

Requires Rust (edition 2024; tested with Rust 1.98), a desktop session, and a
Vulkan-capable graphics driver. No game engine or external shader compiler is
required: the build script compiles WGSL to SPIR-V with Naga.

```powershell
cargo run --release
cargo run --release -- --preset tiny
cargo run --release -- --radius 50000
```

Presets: `tiny` (1 km), `small` (50 km), `large` (1,000 km), `earth` (6,371 km,
the default). All use the same code path. `--help` lists options without opening
a window. The application explicitly uses Vulkan through ash.

| Control | Action |
| --- | --- |
| Left mouse drag | Orbit around the sphere |
| Wheel | Logarithmic orbit zoom; forward/back step in flight |
| Tab / panel button | Switch orbit and free flight |
| Hold right mouse in flight | Mouse look |
| WASD | Move along camera forward/right axes |
| Q / E | Move along camera down/up axes |
| Shift / Ctrl | Multiply flight speed by 10 / 0.1 |
| Home | Reset to orbital view |
| Escape | Exit |

Flight speed scales with reference altitude. Focus loss clears held inputs and
releases the cursor. Flight is unconstrained: collision is not implemented.

## Implemented

- winit desktop window and event loop; ash Vulkan instance, graphics/present
  queue, swapchain, depth image, render pass, pipeline, buffers and resource cleanup.
- A lit Earth-radius cube-sphere test mesh, six faces at 64 × 64 quads per face.
- Infinite reversed-Z projection, depth clear 0, comparison GREATER, 5 cm near plane.
- `f64` absolute CPU positions. Camera position is subtracted **before** converting
  vertices to `f32`; the GPU view matrix contains rotation only.
- Orbit and free-flight cameras, configurable radius and four presets.
- egui debug panel with measured CPU frame interval, GPU name, coordinates,
  radius, altitude, flight speed, mesh counts and the camera's spatial cell address.
- Canonical `i64` 10 cm addresses, floor semantics for negative positions, cell
  centres, and rejection of non-finite/out-of-range positions.
- Resize/minimize handling, optional Vulkan validation, and scripted GPU smoke test.

The test mesh is disposable renderer geometry, not the world definition. It
does not represent stored matter cells. The cell address displayed is the cell
containing the **camera**, not a terrain ray pick or a density/material sample.

## Deliberate foundation limitations

The fixed mesh becomes visibly faceted near ground. Camera-relative arithmetic
has unit coverage at sub-metre Earth-scale distances, but smooth, jitter-free
ground rendering is not claimed yet. Small near-camera patches and adaptive LOD
are needed before milestone 1 can be signed off visually.

One frame is in flight. Host-coherent vertex data is rewritten each frame to
exercise CPU origin subtraction; this is bounded but intentionally temporary.
Later patches should retain local vertices and upload relative origins. There
are no terrain generation jobs yet, so no worker pool or generation caches.

Unimplemented: procedural density/materials, seed/version identity, override
provider, quadtree refinement and LOD transitions, atmosphere, volumetric
clipmaps/meshing, ray picking, cell-grid rendering, GPU timing and memory profiling.
The UI explicitly labels pending features instead of reporting simulated values.

## Verification

```powershell
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo run -- --smoke-test 120
cargo run -- --validation --smoke-test 120
```

Validation requires the Vulkan SDK's `VK_LAYER_KHRONOS_validation`. Requesting
it when unavailable fails explicitly. Without `--validation`, no SDK is required.
The smoke test presents at least 120 frames, exercises orbit and flight modes,
relocates to the opposite side at 0.5 m reference altitude, resizes twice and
exits. It verifies execution, not visual quality. Also manually check drag,
wheel, flight, focus loss, minimize/restore, high-DPI scaling and close behavior.

Unit tests cover negative coordinates, invalid inputs, adjacent Earth-scale
cells on both sides of the planet, camera-relative precision, reverse depth,
camera mode continuity, shared cube edges and command-line validation.

Initial verification on Windows / NVIDIA GeForce RTX 3060: all seven tests,
formatting, Clippy with warnings denied, and the 120-frame GPU smoke test passed.
The validation-layer run could not proceed because the Khronos layer is not
installed on this machine; validation cleanliness is not yet established.

## Next increments

1. Validate the foundation on target hardware; move to patch-local geometry and
   verify visual precision near the surface (milestone 1).
2. Add deterministic seed/versioned macro sampling and bounded cube-sphere
   quadtree refinement, neighbour balancing and crack treatment (milestone 2).
3. Add canonical volumetric `PlanetField`, cell materials and an empty override
   provider, with repeated/order-independent query tests (milestone 3).
4. Introduce asynchronous bounded near-field bricks at 0.10 m and coarser scales,
   density meshing and consistent surface transitions (milestone 4).
5. Add ray-based cell inspection, repeated-query verification and debug grid;
   then profile rapid descent/ascent and eviction (milestones 5–6).

Dependency integration references: [ash](https://github.com/ash-rs/ash) and
[egui-ash-renderer](https://github.com/adrien-ben/egui-ash-renderer). Cargo.lock
pins the mutually compatible versions used by this build.
