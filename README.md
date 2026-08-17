# Shapes of War (Rust)

The Rust rewrite of *Shapes of War* — a real-time 4X strategy game rebuilt
around **continuous time** (no turn-slicing). This is a fresh project, versioned
from `v0.0.1`.

A Cargo workspace with two crates:

- **`game/`** (`shapes_of_war`) — the game itself. Bevy + ECS. So far it
  generates a natural world (tectonic plates, erosion, climate, biomes) and
  renders it as a relief-shaded map.
- **`launcher/`** (`shapes_of_war_launcher`) — a small launcher that checks
  GitHub for a newer `ShapesOfWar.exe`, downloads it, and launches it (the same
  auto-update behaviour as the original Python launcher).

## Run

```sh
cargo run -p shapes_of_war          # the game (worldgen + map viewer)
cargo run -p shapes_of_war_launcher # the launcher
```

## Plan

See [`RUST_PORT_PLAN.md`](RUST_PORT_PLAN.md) for the port strategy, the
continuous-time execution model, and the milestone roadmap (M0 walking
skeleton, M1 worldgen — both done).
