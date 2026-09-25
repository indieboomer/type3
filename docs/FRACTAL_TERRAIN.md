# Increasing terrain complexity toward the surface

Yes: the existing coordinate and LOD architecture supports this. Refinement must
reveal finer frequencies of the same world, rather than change its definition
based on the camera position.

## Why the milestone-2 planet becomes smooth

This follows directly from `Planet::elevation`: its finest noise coordinate is
`direction * 1760`. On an Earth-radius planet, that corresponds to a noise lattice
scale of approximately `6,371,000 / 1760 = 3,620 m`. The other regional scales are
about 14.5 km and 57.9 km. Subdividing triangles below those scales adds samples
to an already smooth function. There is no metre-scale rock shape to discover.

The 2,048-patch budget can also relax refinement when overloaded, and generation
can temporarily lag the camera. These affect sampling quality, but raising the
budget alone cannot create frequencies absent from the field.

## What milestone 3 now provides

The canonical density combines the macro boundary with a finite fractal sum of
3D value-noise layers. On Earth, the noise lattice scale halves each octave:

```text
512, 256, 128, 64, 32, 16, 8, 4, 2, 1, 0.5, 0.25 metres
```

The first amplitude is 12.8 m and subsequent amplitudes multiply by 0.55. The
last amplitude is about 1.78 cm. Maximum absolute displacement before applying
the near-surface envelope is bounded by the geometric series at 28.45 m.
The implied roughness exponent is approximately 0.86. The finest features are
small in amplitude but still add curvature within a metre; a regression test
checks that this detail does not reduce to flat interpolation.

These are noise lattice scales, not a guarantee that arbitrary topology is
resolved at twice the cell size: value noise is not perfectly band-limited.
A 0.10 m volumetric sampler will still require suitable filtering and geometric
error control. Finer-than-cell geometric structure is deliberately excluded.

Smaller planets use a smaller initial scale, `clamp(radius * 0.01, 0.2, 512)` m,
with the same halving rule and a 0.2 m minimum scale. A bounded surface envelope
avoids evaluating these octaves through the entire interior or empty space.
Separate 3D noise bands intersect to form cavities between 20 and 80 m macro
depth on planets of radius at least 1 km; the shell scales down below that radius.
Solid roofs, empty cavities and solid floors are verified along radial queries.

Cell sampling is always full-detail, camera-independent and stateless. Seed,
generation version, radius and cell address identify the result. There is no
global voxel array. Generation version 2 preserves the version-1 macro landscape
and adds the volumetric definition; historical version selection is not exposed.

## Rendering progress and remaining improvements

Milestone 4 now renders this field in nine nested brick levels down to 0.10 m.
Marching tetrahedra extracts full 3D surfaces, including cavity walls. Shared
edge roots are refined against density and normals come from its gradient.
The outermost region spans 819.2 m and morphs toward the macro approximation;
overlap dithering and entry fading bridge the rendered representations. The
near-ground radial guard now locates the outer density crossing. Noclip permits
debug underground inspection; full cave-wall collision is still pending.

The design and remaining rendering work are:

1. Bounded local density bricks and volumetric extraction are implemented. Cache
   eviction and cancellation keep work local to the current viewpoint.
2. For coarser render representations, evaluate filtered approximations of the
   same seeded density. Omit or attenuate noise scales smaller than roughly
   2–4 sample spacings. Smoothly blend filter weights as spacing changes;
   cell queries must continue to use the unchanged full field.
3. The implemented coarse volume extends far beyond the roughly 28.45 m
   displacement bound. Its outer density converges to the macro boundary before
   the overlap. This transition is a render approximation, never a cell override.
4. Match shared boundary samples, transition geometry and neighboring LODs.
   Differing octave counts on adjacent patches need coordinated edge filtering
   and stitching; simply enabling more noise on each finer patch creates cracks.
5. Use displacement/error bounds for LOD requests and bounds, plus hysteresis,
   cancellation of obsolete jobs and gradual mesh transitions to reduce popping.
   Surface shading should use gradients of the field at a compatible scale.

This keeps the number of active samples related to visible detail, rather than
planet volume. A local observer can discover boulders, grooves and cavities
without evaluating those features over an entire continent.

Ridged multifractal layers, domain warping and material-dependent roughness are
reasonable later extensions if ordinary fBm looks too uniform. Their warp and
displacement bounds must be accounted for in LOD, collision and sampling costs.
There is no benefit to infinite recursion: the canonical 10 cm lattice supplies
a deliberate practical endpoint.

## Reference and verification

NVIDIA's [GPU Gems 3 chapter on procedural terrain](https://developer.nvidia.com/gpugems/gpugems3/part-i-geometry/chapter-1-generating-complex-procedural-terrains-using-gpu)
describes combining 3D density, noise octaves, volumetric extraction and different
block resolutions. That supports the approach; the scales and limits above are
derived from this repository's implementation, not performance claims from that
chapter.

Tests cover fixed-cell repeatability, reversed query order, seed variation,
sub-metre variation, displacement bounds, density continuity, shared cube edges,
surface sign crossings and enclosed cavities. Milestone-4 tests also verify
closed cavity meshes, matching brick edges, nested overlap weights, 0.10 m
geometry, cancellation and eviction. Vulkan smoke tests render the volumes.
Transition polish, filtered coarse sampling and full collision remain work for
later increments; current dithering does not guarantee watertight LOD topology.
