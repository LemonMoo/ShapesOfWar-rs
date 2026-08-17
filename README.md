# Shapes of War (Rust)

The Rust rewrite of *Shapes of War* — a real-time 4X strategy game rebuilt
around **continuous time** (no turn-slicing). This is a fresh project, versioned
from `v0.0.1`.

A Cargo workspace with two crates:

- **`game/`** (`shapes_of_war`) — the game itself. Bevy + ECS. It generates a
  natural world (tectonic plates, erosion, climate, biomes), renders it as a
  relief-shaded map, and *lives*: a continuous `SimClock` (fixed 10 Hz,
  1 sim-second = 1 day) drives seasons, and **two realms' towns** produce,
  convert, store and consume continuously — crops on their real growth
  cycle, forestry and mining year-round, the mill→bakery and sawmill chains,
  the Mint striking Gold from mined Gold Ore, per-pool storage that throttles
  production, per-resource spoilage. They **trade**: the capitals build a
  land route (or take an immediate sea route), continuous caravans carry
  goods for gold with a two-phase payment, and each realm's kingdom treasury
  — fed by income tax and a trade tax — funds storage buildings
  (granary/warehouse/vault) that expand its pools.
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
skeleton, M1 worldgen, M2 time + settlement, M3 economy, M4 trade + build —
done).
