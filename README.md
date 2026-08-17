# Shapes of War (Rust)

The Rust rewrite of *Shapes of War* — a real-time 4X strategy game rebuilt
around **continuous time** (no turn-slicing). This is a fresh project, versioned
from `v0.0.1`.

A Cargo workspace with two crates:

- **`game/`** (`shapes_of_war`) — the game itself. Bevy + ECS. So far it
  generates a natural world (tectonic plates, erosion, climate, biomes),
  renders it as a relief-shaded map, and *lives*: a continuous `SimClock`
  (fixed 10 Hz, 1 sim-second = 1 day) drives seasons, and one faction's town
  produces, **converts**, stores and consumes continuously on the map (crops
  on their real growth cycle, forestry year-round, the mill→bakery and
  sawmill chains, post-year luxury trickle, per-pool storage that throttles
  production, per-resource spoilage — all ported from the Python economy).
- **`launcher/`** (`shapes_of_war_launcher`) — a small launcher that checks
  GitHub for a newer `ShapesOfWar.exe`, downloads it, and launches it (the same
  auto-update behaviour as the original Python launcher).

## Run

```sh
cargo run -p shapes_of_war          # the game (worldgen + map viewer + live sim)
cargo run -p shapes_of_war_launcher # the launcher
cargo test -p shapes_of_war         # headless fingerprint + invariant tests
cargo run -p shapes_of_war --example debug_site  # year-by-year state, fixed seed
```

## Plan

See [`RUST_PORT_PLAN.md`](RUST_PORT_PLAN.md) for the port strategy, the
continuous-time execution model, and the milestone roadmap (M0 walking
skeleton, M1 worldgen, M2 time + settlement, M3 economy — done).
