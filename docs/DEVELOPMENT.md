# Implementation status

Milestone 4 adds visible near-field volumetric terrain with 0.10 m finest sampling,
bounded nested bricks, background meshing and overlap transitions to the macro
planet. The canonical density, cell queries, stars and precision foundations
remain in place. Q/E now roll; Space/C provide vertical movement.

The [fractal terrain analysis](FRACTAL_TERRAIN.md) explains why the old terrain
became smooth close up, what detail now exists in the field, and how to render it.

## Run

Requires Rust (edition 2024), a desktop session and a Vulkan-capable driver.
WGSL is compiled to SPIR-V with Naga; no external shader compiler is needed.

```powershell
cargo run --release
cargo run --release -- --preset tiny --seed 42
cargo run --release -- --radius 50000 --seed 928371
cargo run --release -- --sample-cell 0 0 0
cargo run --release -- --seed 42 --sample-cell -63710000 0 0
```

Presets: `tiny` (1 km), `small` (50 km), `large` (1,000 km), `earth` (6,371 km,
the default). All use the same code path. `--help` lists all options.
`--sample-cell X Y Z` prints the world identity, cell centre, signed density,
material, sample source and repeat-query verification without opening Vulkan.
Coordinates are signed integer 10 cm cell addresses, not metres.

| Control | Action |
| --- | --- |
| Left mouse drag in orbit | Rotate around the planet |
| Wheel in orbit | Logarithmic zoom relative to the surface |
| Tab / panel button | Toggle orbit and free flight; flight captures the pointer |
| Mouse in captured flight | Turn in the camera's local axes |
| W / S | Fly forward / backward |
| A / D | Strafe left / right |
| Q / E | Roll left / right about camera forward (about 69 degrees/second) |
| Space / C | Move along camera up / down axes |
| Shift / Ctrl | Multiply speed by 10 / 0.1 |
| Wheel in flight | Adjust speed multiplier logarithmically |
| Escape while captured | Release pointer and stop movement to use the panel |
| Click scene in flight | Capture pointer and resume flight |
| Home | Reset to orbital view |
| Escape while released | Exit |
| Freeze terrain LOD checkbox | Keep the current patch geometry for inspection |
| Fly just above this surface button | Move to 1 m above the density surface and enter flight |
| Volumetric terrain checkbox | Enable/disable near-field geometry |
| Noclip checkbox | Disable the surface guard to inspect underground cavities |

Flight speed follows altitude above the procedural surface; the panel shows the
effective speed including held modifiers. Movement is normalized so diagonals
do not move faster. Focus loss releases the cursor and clears movement keys.
Near ground, the radial guard finds the outer density crossing and maintains
0.5 m clearance above it. This is not swept collision against arbitrary cave
walls. Noclip disables the guard for underground inspection. Roll changes the
local up/right axes without translating the camera or changing its heading.

Tab, Home and Escape are reserved application shortcuts and bypass egui's
keyboard consumption. The panel explicitly shows whether flight controls are
active, paused, or waiting for Tab.

## Terrain and rendering

- Seed plus explicit generation version determines stateless 3D value noise.
  All patches sample the same sphere-direction function, without UV textures
  or longitude/pole seams. The seed is configurable with `--seed`.
- Six cube faces have adaptive quadtrees, with 17 by 17 samples per patch.
  Projected sample spacing controls refinement; a bounded budget relaxes the
  threshold if necessary. Neighbor balancing includes cube-face boundaries and
  enforces at most one level of difference. Radial skirts cover mixed-LOD cracks.
- At most 2,048 active patches and depth 24. One worker generates mesh updates;
  request/result channels are bounded, and the patch cache retains only active
  patches. The render loop continues displaying the previous mesh while work
  completes, and requests the latest viewpoint when the worker becomes available.
- Basalt basins, sand, soil, granite and ice have surface colors derived from
  elevation, latitude and slope. Dark basins are exposed terrain, not simulated
  oceans. Terrain normals come from a fixed-scale derivative of the same field.
- A deterministic 2,200-star celestial background uses emissive quads with
  rotation only. Stars have no camera-translation parallax and render behind
  all planet geometry with reversed-Z depth.
- CPU positions and sampling use `f64`; camera subtraction occurs before `f32`
  conversion. The GPU view matrix contains rotation only. Infinite reversed-Z
  uses a 5 cm near plane, depth clear 0 and comparison GREATER.
- The panel reports seed/version, reference and surface altitude, flight speed,
  patch count, maximum depth, generation time and worker activity. Its cell
  coordinate identifies the camera's 10 cm cell and shows its actual density and
  material, not a terrain ray pick. Expand **Query any 10 cm cell** to edit XYZ,
  select the camera/centre/a point 100 m below the macro surface, and sample or
  repeat the query. On very small planets that depth is capped at half the radius.

## Canonical matter

- `PlanetField` exposes density at an `f64` position, 3D material sampling and
  `sample_cell(CellCoord)`. Cells sample their centres. Density uses `f64` and is
  a signed field value in metre-like units: positive solid, zero/negative empty.
  It is neither physical mass density nor an exact signed-distance function.
- Generation version 2, planet seed, radius and integer cell address define the
  result. The macro terrain stays compatible with version 1. There is no mutable
  RNG, global cell storage, or camera/LOD input to canonical queries.
- Surface detail is a bounded 3D fractal sum. Earth uses twelve noise scales
  from 512 m down to 0.25 m. Cavities occupy a bounded shallow shell, giving empty
  intervals below solid roofs. Deep interior is solid, and the centre has a
  continuous density value independent of direction.
- Materials include air/vacuum, soil, sand, granite, basalt, limestone and ice.
  Surface color selection is shared with the macro renderer; deeper geology is
  a deterministic 3D query. Air is a material classification, not simulated gas.
- `CellOverrideProvider` and `sample_effective_cell` resolve overrides before
  procedural samples and report the source. The app uses `EmptyOverrideProvider`;
  mining and persistence are not implemented.

## Near-field volumes

- Volumes activate within 220 m of the macro surface. Nine levels sample at
  0.10, 0.20, 0.40, 0.80, 1.60, 3.20, 6.40, 12.80 and 25.60 m spacing.
  Their cube widths are 3.2 through 819.2 m. The finest volume covers 3.2 m,
  rather than the README's illustrative 6.4 m; the sampling resolution is 0.10 m.
- Each level has 4 by 4 by 4 bricks of 8 cubed cells. Samples align to integer
  world lattice coordinates before conversion to metres, including negative
  coordinates. The camera-following bounds snap to brick increments. Existing
  mesh bricks are reused until they leave the active set.
- A consistent six-tetrahedron decomposition of each cube extracts the density
  surface. This marching-tetrahedra implementation avoids a large marching-cubes
  case table, supports enclosed cavities and uses shared-edge root refinement.
  Normals come from the density gradient; colors use the field's materials.
- The worker keeps at most 576 cached bricks, one pending request and one result.
  A newer request replaces an obsolete pending request, and generation checks
  cancellation between bricks. Leaving the near-field region clears both the
  displayed volume and worker cache. Density arrays are temporary per-brick
  scratch data; cached results retain disposable meshes, not stored world matter.
- CPU brick origins remain `f64`, vertices are brick-local `f32`, and the origin
  is made camera-relative before upload. Volume GPU capacity is bounded at
  400,000 vertices / 2,400,000 indices in addition to the existing macro capacity.
  Exceeding it reports an error and retains the preceding mesh rather than
  truncating surfaces or growing memory without limit.
- Levels overlap. Complementary screen-space dither masks select the fine/coarse
  surfaces; the coarsest volume fades its density to the macro approximation
  before the outer overlap. Entry also blends over the 160–220 m altitude band
  and ramps over 0.35 seconds. Canonical cell queries remain unchanged.
- The panel reports volume triangles, generation time, retained geometry bytes,
  worker state and cache count. The geometry byte count is not total process or
  GPU memory. At 0.5 m ground clearance the smoke test requires a nonempty 0.10 m
  mesh, renders it, then ascends and verifies volume eviction.

## Current limits

Macro LOD swaps have skirts but no geomorphing; refinement can visibly pop, and very
fast movement can temporarily outpace the worker. Camera-relative geometry and
indices are uploaded through fixed-capacity host-coherent buffers each frame;
resident patch-local GPU buffers and culling are future optimizations. There is
one Vulkan frame in flight. Generation time is CPU worker time, not GPU time.

The volume overlaps are a prototype transition, not topologically stitched
mixed-resolution meshes. Dithering, mismatched silhouettes, aliasing and snapping
can remain visible at transitions, especially during fast movement. Coarse levels
sample the same full field and can miss features below their spacing; octave
filtering and transition-cell meshing are future improvements. Initial generation
can take around a second, with the macro mesh shown until the result is ready.
The radial guard follows the outer density surface, which can differ from a
coarse rendered triangle; full collision and cave navigation remain debug-only.

There is no terrain ray picker, cell-grid renderer, atmosphere rendering or GPU
profiler. Cell queries are available, but the full ray-based inspector remains
milestone 5.

## Verification

```powershell
cargo fmt --check
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings
cargo build --locked --offline
.\target\debug\type3.exe --smoke-test 240
.\target\debug\type3.exe --preset tiny --seed 42 --smoke-test 240
.\target\debug\type3.exe --validation --smoke-test 240
```

The smoke test sends Tab through the keyboard handler with UI consumption set,
checks pointer capture, sends mouse motion through the input handler, and holds W
through the normal frame update while asserting actual camera movement. It then
holds Q through that same path and checks roll without translation or heading
change. It also exercises E, two resizes and relocation
to the opposite side at 0.5 m above the procedural surface. It waits for that
viewpoint's macro and volume meshes, checks 0.10 m geometry, and presents them for
180 frames before returning to orbit. It waits for volume eviction and may exceed
the requested frame count while waiting for generation and the surface hold.

Verified on Windows / NVIDIA GeForce RTX 3060: all 27 tests, formatting, Clippy
with warnings denied, and Earth/tiny Vulkan smoke tests. Tests cover
determinism and query order, seed differences, shared cube edges, bounded and
balanced LOD at poles/edges/corners, sampled mixed-LOD skirt overlap, star
translation invariance, camera movement and speed, precision, and CLI parsing.
Matter tests additionally cover 4,096 cells in reversed query order, arbitrary
interior/space queries, air and vacuum, override precedence, continuity at the
centre and shell boundaries, solid-cavity-solid intervals, fine detail and
extreme cell addresses. Headless queries verify the CLI without a graphics window.
Mesher tests verify shared brick edges, triangle winding and closed inner/outer
cavity surfaces without open mesh edges. Worker tests cover replacement of an
in-flight viewpoint, nonempty 0.10 m geometry, bounded cache and eviction on ascent.
Clipmap bounds and complementary overlap weights are tested on both sides of Earth.

Khronos validation requires `VK_LAYER_KHRONOS_validation`; the attempted run
confirmed it is not installed on this machine. Runtime smoke tests do not
establish visual quality or validation cleanliness. Manually check mouse capture,
focus loss, near-ground flight, rapid descents, cube-face crossings and LOD swaps.

## Next milestones

5. Ray-based cell inspection and cell-grid visualization.
6. Stress testing, transition polish and profiling.
