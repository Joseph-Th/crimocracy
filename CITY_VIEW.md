# City View Core

The city view is a read-only visual projection, not a fifth simulation system or a renderer.
The authoritative world remains the source of truth for neighborhoods, business identities,
business ownership, and game decisions. The rendering adapter owns GPU resources, camera,
selection, highlighting, shaders, and dossiers.

## Current geometry contract

- A stable caller-supplied visual seed determines appearance without reading or consuming
  gameplay RNG. The GUI must retain the seed across saves and reloads for identical visuals.
- Each neighborhood has one independently cullable district chunk, positioned in an
  eight-column city grid by persistent neighborhood ID. This grid is an initial placeholder
  for future actual terrain and road-topology generation, not realistic geography.
- Each district contains a 32-by-32 lot grid and 66 instanced street strips. All lots have
  generated geometry, but only lots assigned to modeled businesses have interactive IDs.
- District business allocation walks IDs in ascending order through a seeded full-cycle
  permutation of lots. A newly created business therefore does not relocate earlier ones.
  The current contract rejects more than 1,024 modeled businesses in a single district.
- Building instances are grouped into six mesh classes, ready to upload as contiguous
  per-district GPU instance arrays. Use one shared mesh/material per class, then draw the
  visible chunks. This is instancing, not skeletal skinning.
- Building picking renders the business's persistent raw ID as an integer into a separate
  business-only picking buffer. Decorative buildings use zero. A pick maps back to a
  BusinessId and then to the authoritative business record. IDs of other entity classes
  require distinct picking buffers/namespaces.
- The generator exposes ground-level pins for camera focus, world labels, and dossier
  positioning. GUI dossiers must enforce player knowledge; being able to select a
  building does not grant knowledge of its hidden owner, rackets, or investigations.

## Performance and limits

The cost of generating the scene is linear in the number of district lots, with ordered
business indexing. GPU drawing is expected to cull whole districts before processing
their batched building instances. No simulation tick should trigger an unconditional city
regeneration. The refresh_neighborhood operation regenerates only a changed district,
atomically replacing its instance arrays and pick mappings. The adapter should reupload
only that district's buffers when modeled businesses change; ownership and ordinary
operational changes do not need geometry regeneration.

This module deliberately does not introduce a graphics framework, 3D assets, full street
topology, GPU rendering, real-time citizen movement, or new persistence fields. Geometry
does not affect line of sight, travel time, policing, or operation resolution. Connecting
those mechanics would require an explicit spatial game-system design and persisted truth.
