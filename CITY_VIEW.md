# City View Core

The city view is a read-only visual projection, not a fifth simulation system or a renderer.
The authoritative world remains the source of truth for neighborhoods, business identities,
business ownership, and game decisions. The rendering adapter owns GPU resources, camera,
selection, highlighting, shaders, and dossiers.

## Current geometry contract

- A stable caller-supplied visual seed determines appearance without reading or consuming
  gameplay RNG. The GUI must retain the seed across saves and reloads for identical visuals.
- Each neighborhood has one independently cullable district chunk, positioned in a
  regular grid by persistent neighborhood ID. `CityScene::generate` retains the
  original eight-column default; `generate_with_columns` accepts 1–32 columns
  for different city shapes (the WebGL2 preview uses four). Both the visual
  seed and column count belong to the presentation configuration and must remain
  fixed across save/restore. Adding neighborhoods does not move earlier
  districts. The grid remains an initial placeholder for future terrain and
  simulation-backed transport topology.
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
  adapter requires an explicit set of disclosed business IDs and emits exact
  little-endian 32-byte records (translation.xyz, scale.xyz, disclosed pick ID
  or zero, variation), one contiguous GPU upload buffer per nonempty building
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

The Rust city-view module deliberately does not depend on a graphics framework or
own 3D assets, full street topology, GPU resources, real-time citizen movement, or
new persistence fields. Geometry does not affect line of sight, travel time,
policing, or operation resolution. Connecting those mechanics would require an
explicit spatial game-system design and persisted truth.

## Interactive WebGL2 preview

Run `cargo run --locked --example city_gui_preview` from the project root and open
`target/agent-output/city_preview/index.html` in a WebGL2-capable browser.
The generated HTML is self-contained: no web server, package manager, remote
scripts, or CDN is needed. Source files are `gui/city_preview/{index.html,styles.css,app.js}`;
the Rust exporter is `examples/city_gui_preview.rs`. The output file lives in the
ignored `target/` directory and can be regenerated whenever the source changes.

The preview renders the actual `CityScene` instance buffers with WebGL2's
`drawArraysInstanced`, keeping GPU instance buffers resident while camera
changes create new visible draw ranges. WebGL2 has no base-instance argument
for this draw operation, so the adapter offsets its per-instance vertex
attributes into the existing buffers instead of uploading geometry again.
The separate depth-tested RGBA8 picking framebuffer encodes all four bytes of
the business ID; the directory acts as the disclosure gate before opening a
dossier. Each district's geometry remains base64-encoded until the camera
first visits it; then the bytes are decoded once and reused for subsequent
GPU uploads. The adapter keeps at most four resident districts where the
viewport permits; it evicts least-recently-used offscreen GPU buffers while
retaining a single CPU copy for re-upload when revisited. If more than four
districts are visible simultaneously, all visible districts remain resident.
Picking draws only buildings and does not overwrite normal-render statistics.
Rendering is demand-driven rather than redrawing continuously while idle.

The camera supports left-drag panning, right-drag or Shift-drag orbit, scroll
zoom anchored to the pointer, Q/E rotation, arrow-key panning, +/- zoom,
and a whole-city overview framed around the currently available map area.
Click a registered building, a map label, or an entry in the searchable
directory to select a property. The property dossier presents the information
deliberately included in the preview's public directory. The center of the
city remains visible behind the contextual panels. Map labels are created only
for visible public properties and removed as they leave the view; screen-space
collision checks and a 24-label limit avoid clutter. The directory caps its
rendered rows at 200 and retains search across the full authorized directory.
Escape dismisses selection even when the search box has focus.

**Disclosure boundary:** the bundled eight-district, 96-property world is a
fictional fixture built with the canonical world insertion APIs. All exported
names and categories are intentionally authored as public. The exporter does
not serialize live intelligence, owner identities, police records, case details,
or unrevealed economic information. An additional unlisted sample property
proves that the binary GPU instance buffers mask its persistent ID to zero;
player-facing exports must explicitly supply their authorized public directory
IDs to `GpuCityChunk::encode`. The browser is a read-only projection, not
a campaign loader or a gameplay command adapter. A future campaign integration
must populate public dossier fields from player-visible information sources,
not from hidden `AppState` truth, and must send gameplay actions through the
canonical validated systems. Browsing a building must never disclose its
hidden records by inference from an unfiltered payload.

The opt-in `python scripts/city_preview_browser_smoke.py` check runs a
headless Chrome/Edge session with an isolated profile and tests real WebGL
draws, directory searching, navigation, GPU picking, dossier visibility,
zoom/orbit, camera reset, and a narrower desktop viewport. It requires
the local `websocket-client` Python package and Chrome/Edge. The script
closes its own browser and removes its temporary profile. Its screenshots
are saved under `target/agent-output/city_preview/`. A passing browser
check verifies functionality in the installed headless browser, not
hardware-GPU frame rate or final production appearance.

For geometry inspection, run `cargo run --locked --example city_view_workbench`.
This produces `target/agent-output/city_view_workbench.ppm`, a 1,600-by-1,000
isometric CPU reference image of the actual generated boxes and road strips.
It is a geometry verification workbench, not the final game renderer.

`cargo run --locked --example city_view_benchmark` measures preparation
of a 64-district sample, its GPU payloads, and repeated close-up draw-plan
construction. This is a CPU diagnostic and does not measure rendering FPS.
