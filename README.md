# Stage 1 – One-Litre Procedural Planet Prototype

## 1. Project Goal

Build a standalone experimental renderer/simulation prototype in **Rust + Vulkan** that demonstrates a single procedural planet whose underlying matter model is addressable at a fixed spatial resolution of:

**10 cm × 10 cm × 10 cm = 1 litre per spatial cell.**

The prototype must allow the user to:

- view the entire planet from space,
- rotate around it,
- zoom continuously from orbital distance toward the surface,
- enter free-flight mode,
- fly over the terrain,
- descend to ground level,
- inspect terrain at very short range,
- identify individual 10 cm spatial cells,
- move back from the surface to orbit without switching maps or loading a different representation of the planet.

The central experiment is NOT merely rendering a procedural spherical terrain.

The purpose is to prove the architecture for a future universe in which every solid astronomical object can conceptually be queried and modified at 10 cm resolution without storing all of its cells.

The planet must therefore be represented as a **procedural volumetric matter field**, not as a conventional stored voxel planet and not merely as a heightmap wrapped around a sphere.

---

# 2. Core Principle

The planet does not consist of stored voxels.

It consists of a deterministic function.

Conceptually:

```rust
sample_matter(planet_seed, cell_coordinate) -> CellSample
```

where:

```rust
struct CellSample {
    density: f32,
    material: MaterialId,
}
```

A cell coordinate identifies an exact 10 cm cube in planet-local space.

For example:

```text
CellCoord(18472131, -8371282, 51982731)
```

always refers to exactly the same litre of space.

Calling `sample_matter()` for this coordinate must always produce the same result for the same:

```text
universe generation version
planet seed
cell coordinate
```

unless a future modification layer overrides it.

No global voxel array may exist.

---

# 3. Non-Negotiable Architectural Rules

These requirements must be treated as invariants.

## 3.1 The canonical world is volumetric

The canonical representation is a 3D matter field:

```text
Matter(x, y, z)
```

and NOT:

```text
Height(x, z)
```

or:

```text
Height(latitude, longitude)
```

A surface-height function may exist as a lower-cost approximation for distant rendering, but it must not become the authoritative representation of the world.

## 3.2 10 cm is the canonical finest spatial resolution

Define:

```rust
const CELL_SIZE_METERS: f64 = 0.1;
```

The world-addressing system must be able to address any 10 cm cell anywhere inside the planet.

The renderer is free to use coarser representations at distance.

The simulation model is not.

## 3.3 Never materialize the complete planet

For an Earth-sized planet, the number of potential cells is astronomically large.

Do not attempt to:

- allocate them,
- enumerate them,
- serialize them,
- pre-generate them,
- create a global octree containing one node per region,
- build a complete mesh.

Only evaluate procedural information needed for the currently observed region.

## 3.4 Distant LOD is an approximation of the same world

LOD systems may reduce spatial resolution.

They must not create independent random terrain.

All levels ultimately derive from the same planet seed and compatible generation functions.

## 3.5 Camera-relative rendering is mandatory

Never submit Earth-scale absolute coordinates as ordinary `f32` vertex positions.

CPU-side astronomical and planetary positions should use `f64`.

GPU geometry close to the camera should be represented relative to a local origin and may then use `f32`.

## 3.6 Future modification must be possible

Mining is NOT part of Stage 1.

However, the sampling architecture must already anticipate:

```rust
sample_effective_cell(cell) =
    modification_layer(cell)
        .unwrap_or_else(|| procedural_cell(cell))
```

Stage 1 may implement `modification_layer` as an empty stub.

Do not bake procedural terrain directly into permanent geometry in a way that makes future modification impossible.

---

# 4. Target Planet

Start with one Earth-scale rocky planet.

Default parameters:

```text
Radius:             6,371,000 m
Diameter:          12,742,000 m
Cell size:                  0.1 m
Seed:                configurable
Rotation:             optional visual feature
Atmosphere:           simple visual atmosphere
```

The radius should be configurable because development will be easier with smaller test planets.

Suggested debug presets:

```text
Tiny       1 km radius
Small     50 km radius
Large   1000 km radius
Earth   6371 km radius
```

The exact same architecture must work for all presets.

Do not build special code paths for the Earth-scale planet.

---

# 5. Coordinate System

Use a hierarchy of coordinate representations.

## Planet-local physical position

CPU:

```rust
struct PlanetPosition {
    meters: DVec3,
}
```

`DVec3` uses double precision.

Planet centre:

```text
(0, 0, 0)
```

## Canonical spatial cell address

```rust
struct CellCoord {
    x: i64,
    y: i64,
    z: i64,
}
```

Conversion:

```rust
cell.x = floor(position.x / CELL_SIZE_METERS)
cell.y = floor(position.y / CELL_SIZE_METERS)
cell.z = floor(position.z / CELL_SIZE_METERS)
```

An Earth-sized planet requires only roughly ±64 million cells from the centre along each axis, so signed 64-bit integer coordinates provide enormous safety margin.

## GPU coordinates

Rendering should use:

```text
camera-relative coordinates
```

For every mesh or patch:

```text
gpu_position = absolute_position - render_origin
```

Send patch-local geometry as `f32`.

Keep its high-precision origin on the CPU.

The active render origin should follow the camera.

---

# 6. Planet Generation Model

Create a clean interface:

```rust
trait PlanetField {
    fn sample_density(&self, position_m: DVec3) -> f64;
    fn sample_material(&self, position_m: DVec3) -> MaterialId;
    fn sample_cell(&self, cell: CellCoord) -> CellSample;
}
```

Generation must be stateless and deterministic.

Avoid hidden mutable RNG state.

Randomness must derive from:

```text
seed + spatial coordinates + generation layer identifier
```

---

# 7. Separate Macro Surface From Full Volume

For performance, expose two related functions.

## Macro surface

```rust
fn surface_radius(direction: DVec3) -> f64;
```

This produces an inexpensive estimate of the main planet surface.

It is used for:

- orbital rendering,
- regional rendering,
- LOD selection,
- patch bounds,
- estimating altitude.

Example conceptual structure:

```text
base radius
+ continental-scale noise
+ mountain noise
+ ridge noise
+ erosion-like modulation
+ regional detail
```

Noise must operate over a 3D unit sphere direction.

Do not generate terrain using latitude/longitude textures that create seams at longitude boundaries or the poles.

## Full volumetric field

```rust
fn density(position: DVec3) -> f64;
```

Conceptually:

```text
surface =
    base_radius
    + macro_elevation(direction)

depth =
    surface - length(position)

density =
    depth
    + volumetric_rock_noise(position)
    + geological_features(position)
    - cave_field(position)
```

Interpretation:

```text
density > 0     solid
density < 0     empty / atmosphere
density ≈ 0     surface boundary
```

This allows future support for:

- caves,
- arches,
- overhangs,
- tunnels,
- deep geology,
- ore bodies,
- artificial structures.

Stage 1 does not need sophisticated geology.

It DOES need the volumetric architecture.

---

# 8. Initial Procedural Terrain

Implement enough procedural variation to make the planet visually convincing.

Suggested layers:

### Continental scale

Very low-frequency noise creating large regions of high and low terrain.

### Mountain scale

Ridged noise creating mountain chains.

### Regional terrain

Medium-frequency noise generating hills and valleys.

### Local terrain

Higher-frequency noise producing small surface variation.

### Volumetric detail

Low-amplitude 3D density noise near the surface.

### Optional caves

Simple 3D cave noise may be added near the surface.

Caves are useful because they immediately verify that the planet is not only a heightmap.

Even one visible cave system would be an excellent Stage 1 proof.

Do not attempt realistic geological simulation yet.

---

# 9. Materials

Use a small initial material set.

For example:

```rust
enum MaterialId {
    Vacuum,
    Air,
    Soil,
    Sand,
    Granite,
    Basalt,
    Limestone,
    Ice,
}
```

Material selection may depend on:

```text
depth below local surface
latitude
elevation
slope
temperature approximation
3D geological noise
```

For Stage 1, materials primarily affect surface colour.

Do not build inventory or mining systems.

However, the material belongs to the **cell**, not merely to the rendered triangle.

---

# 10. Rendering Architecture

Use three conceptual rendering regimes.

They must blend continuously.

## A. Orbital planet renderer

Used when far from the planet.

Render the planet using relatively low-resolution spherical geometry.

The shape should already contain large-scale procedural elevation.

At extreme distances, very fine terrain does not matter.

Requirements:

- Earth-scale radius,
- directional sunlight,
- basic physically plausible shading,
- optional atmospheric halo,
- no visible spherical UV seam,
- stable depth precision.

## B. Surface quadtree / cube-sphere LOD

Use a **cube-sphere** representation.

Represent the planet using six logical cube faces projected onto a sphere.

Each cube face owns a quadtree of terrain patches.

A patch subdivides when projected geometric error exceeds a screen-space threshold.

Example:

```text
Cube face
    patch
        child 0
        child 1
        child 2
        child 3
```

Each patch samples `surface_radius(direction)`.

Patch resolution can remain relatively small, for example:

```text
17×17
33×33
or
65×65 vertices
```

with quadtree depth supplying detail.

Requirements:

- no visible cracks between LOD levels,
- no seams between cube faces,
- neighbouring patch LOD difference limited to one level,
- skirts or stitching may initially be used if necessary.

Do not generate one giant mesh.

## C. Near-field volumetric renderer

When sufficiently close to the terrain, transition to a true volumetric representation based on the 3D matter field.

Use a nested voxel clipmap or sparse brick system.

The finest level MUST eventually reach:

```text
0.10 m sampling resolution
```

Example initial clipmap:

```text
Level 0    0.10 m cells     ~6.4 m region
Level 1    0.20 m cells    ~12.8 m region
Level 2    0.40 m cells    ~25.6 m region
Level 3    0.80 m cells    ~51.2 m region
Level 4    1.60 m cells   ~102.4 m region
```

Exact values may be tuned.

The important property is:

```text
near camera -> true 10 cm lattice
distance increases -> exponentially coarser sampling
```

Do NOT require 10 cm resolution over kilometres of terrain.

That would defeat the entire architecture.

---

# 11. Near-Field Meshing

For Stage 1, prefer a robust implementation over an ideal final solution.

Recommended first implementation:

**Marching Cubes over sparse/nested volumetric chunks.**

Possible future replacement:

**Dual Contouring / Surface Nets**

if sharper artificial geometry becomes important.

Do not generate one cube mesh per solid cell.

The cell grid represents matter.

The visual surface should be extracted from the density field.

The player should therefore see smooth or semi-smooth geological terrain while the underlying world remains 10 cm addressable.

Add an optional debug mode showing:

```text
10 cm cell grid
cell boundaries
cell centre
material
density
CellCoord
```

This debug view is important for proving that the one-litre spatial model actually exists.

---

# 12. LOD Transition Strategy

The planet must visually behave like ONE object.

A user should be able to:

```text
orbit
↓
approach continent
↓
approach mountain
↓
fly through valley
↓
hover one metre above rock
```

without an obvious mode switch.

The implementation may internally transition through multiple representations.

Hide transitions using:

- overlapping LOD zones,
- geomorphing,
- cross-fading where appropriate,
- consistent procedural sampling,
- matching boundary positions.

Do not solve every transition perfectly in the first implementation.

But the architecture must not require a loading screen or separate surface map.

---

# 13. Precision Strategy

This is critical.

Do not rely on world-space `f32`.

At Earth radius:

```text
~6,371,000 metres
```

ordinary single-precision coordinates cannot represent 10 cm changes reliably.

Use:

```text
CPU planet positions: f64
cell coordinates:     i64
GPU local geometry:   f32
```

Each renderable patch should contain:

```rust
struct RenderPatch {
    origin: DVec3,
    local_vertices: Vec<Vec3>,
}
```

Before drawing:

```text
camera_relative_origin =
    patch.origin - camera.position
```

Only small camera-relative numbers should reach most vertex calculations.

Use reversed-Z depth buffering.

Avoid extremely small near planes.

---

# 14. Camera System

Implement two camera modes.

## Orbit camera

Controls:

```text
LMB drag     rotate around planet
mouse wheel  zoom
RMB drag     optional pan/orbit adjustment
```

It should support everything from:

```text
multiple planet radii away
```

to:

```text
several kilometres above terrain
```

## Free-flight camera

Controls:

```text
WASD        movement
mouse       orientation
Q/E         down/up or roll
Shift       faster
Ctrl        slower
```

Implement logarithmic or altitude-aware velocity scaling.

Example speed range:

```text
0.1 m/s
1 m/s
10 m/s
100 m/s
1 km/s
100 km/s
...
```

The camera must allow a practical trip from orbit to ground without spending minutes holding the forward key.

Show current speed.

---

# 15. Altitude

Do not define altitude simply as:

```text
length(position) - base_radius
```

because mountains exist.

Implement both:

```text
altitude_from_reference_radius
```

and an approximate:

```text
altitude_above_surface
```

Surface altitude may initially use the macro surface function.

---

# 16. Debug UI

Use a small debug interface.

Display at minimum:

```text
FPS
frame time
camera position
distance from planet centre
reference altitude
surface altitude
camera speed

planet seed

active surface patches
quadtree depth
near-field active bricks/chunks
current finest resolution

CellCoord under cursor
material under cursor
density
distance to sampled cell

CPU generation time
GPU frame time
mesh generation queue size
memory usage
```

Add toggles for:

```text
wireframe
LOD colour visualization
quadtree patch boundaries
near-field clipmap bounds
10 cm cell grid
freeze LOD
atmosphere
volumetric terrain
```

Debug visualization is part of the prototype, not optional polish.

---

# 17. Cell Inspector

Implement ray picking.

When pointing at nearby terrain, display something like:

```text
Cell:
X  18371822
Y -29812711
Z  51287391

World centre:
(1837182.25, -2981271.15, 5128739.15) m

Resolution:
0.10 m

Material:
Granite

Density:
+0.382

Source:
Procedural

Planet seed:
928371
```

Add a key to repeatedly query the same cell.

The returned result must remain identical.

This is one of the main acceptance tests of Stage 1.

---

# 18. Threading

Terrain generation must not block the render loop.

Use separate work queues for:

```text
LOD requests
procedural sampling
near-field brick generation
meshing
GPU upload
```

CPU terrain generation may use a worker pool.

The render thread should consume completed resources.

Prioritize jobs according to:

1. visibility,
2. distance,
3. screen-space importance,
4. camera motion direction.

Discard obsolete low-priority generation jobs when the camera moves elsewhere.

---

# 19. Vulkan Structure

Keep Vulkan code separate from world generation.

Suggested top-level project structure:

```text
src/
    main.rs

    app/
        mod.rs
        input.rs
        camera.rs

    renderer/
        vulkan.rs
        device.rs
        swapchain.rs
        pipeline.rs
        buffers.rs
        textures.rs
        depth.rs
        atmosphere.rs
        planet_renderer.rs
        debug_renderer.rs

    world/
        mod.rs
        coordinates.rs
        cell.rs
        material.rs
        hash.rs

    planet/
        mod.rs
        config.rs
        field.rs
        noise.rs
        macro_surface.rs
        volume.rs
        materials.rs

    lod/
        mod.rs
        cube_sphere.rs
        quadtree.rs
        patch.rs
        scheduler.rs

    voxel/
        mod.rs
        brick.rs
        clipmap.rs
        marching_cubes.rs

    ui/
        mod.rs
        debug_ui.rs
        cell_inspector.rs
```

Exact names may change.

Maintain strong separation between:

```text
world definition
procedural generation
LOD selection
mesh generation
rendering
camera
debug UI
```

---

# 20. Suggested Rust Stack

Prefer:

```text
ash              Vulkan bindings
winit            window/event handling
gpu-allocator    Vulkan memory allocation
glam             vector/matrix math
egui             debug UI
rayon            CPU job parallelism if useful
bytemuck         GPU structure conversion where appropriate
tracing          profiling/logging
```

Do not introduce a full game engine.

This project is specifically intended to investigate the architecture directly.

---

# 21. Procedural Determinism

Create an explicit:

```rust
struct GenerationVersion(u32);
```

World identity should effectively be:

```text
GenerationVersion
PlanetSeed
CellCoord
```

Tests must verify:

```rust
sample(cell) == sample(cell)
```

across repeated calls.

Also verify neighbouring cell queries do not depend on query order.

Never use sequential RNG calls for terrain generation.

Use coordinate hashing or deterministic spatial noise.

---

# 22. Future Modification Layer

Do NOT implement actual mining yet.

However, define the interface now.

For example:

```rust
trait CellOverrideProvider {
    fn get_override(&self, cell: CellCoord) -> Option<CellSample>;
}
```

Then:

```rust
fn sample_effective_cell(cell: CellCoord) -> CellSample {
    overrides
        .get_override(cell)
        .unwrap_or_else(|| planet.sample_cell(cell))
}
```

For Stage 1:

```text
overrides = EmptyOverrideProvider
```

This ensures that Stage 2 can introduce mining without rewriting planet generation.

Eventually the override provider may support:

```text
single-cell modifications
compressed regions
CSG operations
constructed matter
destroyed matter
```

But none of these should be implemented now.

---

# 23. Explicitly Out of Scope

Do NOT implement:

- multiple planets,
- solar systems,
- galaxies,
- agents,
- NPCs,
- mining,
- building,
- inventories,
- vehicles,
- orbital mechanics,
- realistic gravity simulation,
- fluid simulation,
- erosion simulation,
- realistic tectonics,
- weather,
- vegetation,
- procedural cities,
- multiplayer,
- persistence of modifications.

These are later problems.

Stage 1 is about proving spatial scale and representation.

---

# 24. Required Visual Result

The prototype should eventually allow the following continuous demonstration:

### Scene 1

Start with the full planet visible against space.

Rotate around it.

Continents, mountains and large terrain features should be visible.

### Scene 2

Zoom toward one region.

Terrain detail progressively increases.

No loading screen.

### Scene 3

Switch to free-flight.

Fly above mountains and valleys.

Move at aircraft-like speed.

### Scene 4

Descend toward terrain.

Local volumetric representation becomes active.

Small geological details appear.

### Scene 5

Hover approximately 1–2 metres above a rock surface.

Enable cell-grid debug mode.

The user can now see or inspect the underlying 10 cm spatial lattice.

### Scene 6

Point at one litre cell.

Display its exact integer address and material.

Move away.

Return to the location.

Query it again.

The result must be identical.

### Scene 7

Accelerate away from the surface.

Fly back to orbital altitude.

The local volumetric representation disappears from memory.

The planet continues to exist procedurally.

This sequence constitutes the core demonstration.

---

# 25. Development Milestones

## Milestone 0 – Vulkan foundation

Implement:

```text
window
Vulkan instance/device
swapchain
depth buffer
basic shader pipeline
camera
basic debug UI
```

Render a simple sphere.

Do not begin procedural terrain before this is stable.

## Milestone 1 – Precision-safe Earth-sized sphere

Create an Earth-radius sphere.

Verify:

```text
orbit navigation
free-flight
camera-relative rendering
reversed-Z
f64 CPU position
```

Test movement from millions of metres away to less than one metre from a surface without visible coordinate jitter.

This milestone is extremely important.

## Milestone 2 – Procedural macro planet

Implement:

```text
planet seed
cube-sphere
procedural surface radius
quadtree LOD
mountains
continents
regional detail
basic materials
```

Achieve seamless navigation from orbit to low-altitude flight.

## Milestone 3 – Volumetric matter field

Implement the canonical:

```text
3D density field
3D material query
CellCoord
sample_cell()
```

At this point the planet stops being merely a procedural spherical terrain renderer.

Verify arbitrary internal coordinates can be queried.

For example:

```text
surface       -> rock/air boundary
100 m deep    -> rock
10 km deep    -> rock
planet centre -> deep interior material
outside       -> air/vacuum
```

## Milestone 4 – Near-field voxel clipmap

Implement nested local volumetric levels.

Reach true:

```text
10 cm sampling
```

near the camera.

Generate local geometry from the density field.

Transition between normal surface LOD and volumetric rendering.

## Milestone 5 – Cell inspector

Implement:

```text
ray picking
CellCoord display
density inspection
material inspection
10 cm debug grid
```

This proves the central concept.

## Milestone 6 – Stability and profiling

Stress test:

```text
rapid orbital descent
rapid ascent
high-speed surface flight
moving across cube-face boundaries
moving across terrain patch boundaries
teleporting to opposite side of planet
repeated LOD regeneration
```

Remove memory leaks and unbounded caches.

---

# 26. Important Tests

Create automated tests wherever possible.

## Coordinate test

Convert:

```text
meters -> CellCoord -> cell centre
```

and verify expected results, including negative coordinates.

## Determinism test

Query thousands of random cells twice.

Results must match exactly.

## Query-order test

Query cells in different orders.

Results must remain identical.

## Planet-scale precision test

Query two adjacent 10 cm cells at approximately Earth radius.

They must remain distinguishable.

## Cube-face seam test

Surface sampling across cube-sphere edges must be continuous.

## LOD seam test

Neighbouring patches with different LODs must not expose holes.

## Planet-opposite-side test

Move camera to approximately:

```text
(+R, 0, 0)
```

then:

```text
(-R, 0, 0)
```

Near-field precision must remain equivalent.

---

# 27. Performance Philosophy

The relevant complexity metric is NOT:

```text
number of possible cells in the planet
```

It is:

```text
number of cells/patches currently evaluated or materialized
```

The architecture should make the cost of rendering an Earth-sized planet roughly related to:

```text
visible screen complexity
+
active local volumetric region
```

rather than planetary volume.

The system should never iterate through empty or unseen planetary volume.

---

# 28. Memory Philosophy

Use bounded caches.

For example:

```text
surface patch cache
near-field brick cache
mesh cache
GPU buffer cache
```

All caches must have eviction policies.

Flying around the planet for one hour must not cause memory usage to grow indefinitely.

A region that has never been modified should always be safe to discard.

It can be regenerated from the seed later.

---

# 29. Definition of Success

Stage 1 succeeds if a developer can run the application and honestly state:

> This planet is Earth-scale. Its matter is procedurally addressable as deterministic 10 cm × 10 cm × 10 cm spatial cells. The planet is never globally stored at that resolution. I can move seamlessly from orbit to the ground, inspect an individual one-litre cell, leave the region, discard its detailed representation, return later, and obtain the same world again.

The prototype does NOT need to prove that every future feature is easy.

It needs to prove that the fundamental spatial representation is valid.

---

# 30. What Not To Fake

Codex should treat the following shortcuts as architecture failures:

### Do not create a normal spherical heightmap and merely label it a voxel planet.

### Do not store a high-resolution planet texture representing all terrain.

### Do not use 10 cm only as a visual texture scale.

### Do not make orbital terrain and local terrain independently generated worlds.

### Do not use `f32` absolute world coordinates.

### Do not pre-generate large volumes of internal planetary cells.

### Do not make terrain generation depend on traversal or RNG call order.

### Do not create a separate "surface level" loaded after approaching the planet.

The fundamental experiment is specifically intended to test whether all of these shortcuts can be avoided.

---

# 31. First Implementation Priority

Optimize for architectural proof rather than visual polish.

The first convincing build should contain:

```text
1 procedural planet
1 star light source
basic atmosphere
cube-sphere LOD
mountains and regional terrain
volumetric MatterField
near-field volumetric clipmap
10 cm finest cells
cell inspector
orbit camera
free-flight camera
debug UI
```

No gameplay is required.

The entire application is essentially an interactive **planet-scale matter browser**.

Once this works, it becomes the foundation for the much larger experiment:

```text
planet
→ destructible planet
→ many astronomical bodies
→ solar system
→ universe
→ individual agents
→ civilization-scale agent simulation
```

But those later stages must not influence implementation scope beyond ensuring that Stage 1 does not architecturally prevent them.
