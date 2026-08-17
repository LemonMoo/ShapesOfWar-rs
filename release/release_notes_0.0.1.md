# v0.0.1

The first release of the Rust rewrite — a fresh project, versioned from
`v0.0.1`, rebuilt around **continuous time** (no turn-slicing).

## What's in it

- **Natural world generation** — tectonic plates (mountain ranges, rifts,
  island arcs), hydraulic/thermal erosion, rivers and lakes, and a climate
  model (latitude rainfall, rain shadows, temperature lapse) that grades into
  jungles, forests, taiga, tundra, deserts and savannah.
- Rendered as a **relief-shaded map** (hillshade + coastal shelf).
- **The launcher** — a separate `ShapesOfWarLauncher.exe` that checks GitHub
  for a newer `ShapesOfWar.exe`, downloads it, and launches it (same behaviour
  as the original Python launcher).

This is the walking-skeleton + worldgen milestones (M0 + M1). Gameplay
(settlements, economy, trade, war) comes next.

## Run

- Drop `ShapesOfWarLauncher.exe` in a folder and run it — it fetches and
  launches the latest `ShapesOfWar.exe` on first start.
- Or run `ShapesOfWar.exe` directly to see the generated world.

---
**Milestones M0 + M1.**
