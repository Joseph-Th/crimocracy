# City View Core

The city view is a read-only visual projection, not a fifth simulation system or a renderer.
The authoritative world remains the source of truth for neighborhoods, business identities,
business ownership, and game decisions. The rendering adapter owns GPU resources, camera,
selection, highlighting, shaders, and dossiers.

## Current geometry contract

- A stable caller-supplied visual seed determines appearance without reading or consuming
  gameplay RNG. The GUI must retain the seed across saves and reloads for identical visuals.
- Each neighborhood has one independently cullable district chunk, positioned in an
  eight-column city grid by persistent neighborhood ID. This regular city plan is an
  initial placeholder for future terrain and simulation-backed transport topology.
- Each district contains a 32-by-32 lot grid, 4-by-4-lot street blocks, 10-meter
  arterial roads, and seed-varied 3-meter alleys. District road strips connect across
  the 24-meter district margins. Generated building footprints respect road setbacks.
  All lots have geometry, but only lots assigned to modeled businesses have interactive IDs.
- District business allocation walks IDs in ascending order through a seeded full-cycle
  permutation of lots. A newly created business therefore does not relocate earlier ones.
  The current contract rejects more than 1,024 modeled businesses in a single district.
- Building instances are grouped into six mesh classes and sorted by 8-by-8-lot
  sector. Sixteen sectors per district expose contiguous per-mesh draw ranges
  within the same instance buffers, so a distant camera can draw each district
  in one batch per mesh and a close camera can cull entire sectors without
  duplicating geometry. `CityScene::draw_plan` produces a small list of
  buffer-range draw commands for either mode and merges contiguous sector
  ranges, so full-coverage sector draws collapse back to the same calls as a
  whole-district view. The `GpuCityChunk::encode`
  adapter emits exact little-endian 32-byte records (translation.xyz, scale.xyz,
  pick ID, variation), one contiguous GPU upload buffer per nonempty building
  mesh and one road buffer. Use instance step mode and one mesh/material per
  class. This is instancing, not skeletal skinning.
- Building picking renders the business's persistent raw ID as an integer into a separate
  business-only picking buffer. Decorative buildings use zero. A pick maps back to a
  BusinessId and then to the authoritative business record. IDs of other entity classes
  require distinct picking buffers/namespaces.
- The generator exposes ground-level pins for camera focus, world labels, and dossier
  positioning. GUI dossiers must enforce player knowledge; being able to select a
  building does not grant knowledge of its hidden owner, rackets, or investigations.

## Performance and limits

The cost of generating the scene is linear in the number of district lots, with ordered
business indexing. Viewport culling binary-searches district rows and visits only
chunks in the overlapping rows. Near-camera culling further restricts building
draw ranges to intersecting sectors. GPU drawing should cull whole districts before
processing their batched building instances. No simulation tick should trigger an unconditional city
regeneration. The refresh_neighborhood operation regenerates only a changed district,
atomically replacing its instance arrays and pick mappings. The adapter should reupload
only that district's buffers when modeled businesses change; ownership and ordinary
operational changes do not need geometry regeneration.

This module deliberately does not introduce a graphics framework, 3D assets, full street
topology, GPU rendering, real-time citizen movement, or new persistence fields. Geometry
does not affect line of sight, travel time, policing, or operation resolution. Connecting
those mechanics would require an explicit spatial game-system design and persisted truth.

For geometry inspection, run `cargo run --locked --example city_view_workbench`.
This produces `target/agent-output/city_view_workbench.ppm`, a 1,600-by-1,000
isometric CPU reference image of the actual generated boxes and road strips.
It is a geometry verification workbench, not the final game renderer.

`cargo run --locked --example city_view_benchmark` measures preparation
of a 64-district sample, its GPU payloads, and repeated close-up draw-plan
construction. This is a CPU diagnostic and does not measure rendering FPS.
