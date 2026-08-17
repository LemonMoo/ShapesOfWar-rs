# Rust Port & the Continuous-Time Core (plan)

**Theme: rebuild the game around *continuous time*, not a sliced turn.** The
Python game is a turn-based 4X whose "real-time" is a day sliced across frames
(`resources.advance_turn` runs one atomic day; `turn_runner` then *slices* that
day's phases across frames so it merely *looks* real-time). The Rust rewrite
keeps the game's **design** — worldgen, species, economy, trade, governance,
combat — but rebuilds it as a genuine continuous simulation: per-entity clocks,
per-second rates, and no atomic "day."

Decisions locked with the user:

1. **Bevy** (the Rust ECS engine) — `bevy = "0.19"`.
2. **2D top-down, with 2.5D in mind.** Elevation is a first-class value in the
   data model from day one; the first pass renders it as a top-down grid with
   height as a depth cue, and a later milestone raises it into a true
   angled/isometric 2.5D presentation (Bannerlord-style camera over a grid,
   not full 3D models).
3. **Walking skeleton first** — a fixed-timestep loop, a generated map, and a
   few entities ticking continuously — *before* porting any 4X system.

---

## 1. The actual problem being solved

The current "real-time" is a presentation trick over a turn-based sim. The
artifacts are baked into `day_steps`' own contract: *"a phase is the atom, and
the world is coherent at every yield"* — and *"it may not stop inside one."*
Concretely: a "day" is still the quantum; rates are per-turn; a caravan takes
"N turns," a build takes "N turns"; nothing can be interrupted mid-phase.

A true real-time game (Mount & Blade, Bannerlord) has no day quantum. Time is a
continuous float; every actor carries its own clock; movement integrates
speed × dt; production accrues rate × dt; a caravan halfway along a road can be
raided mid-trip; a building can be half-finished. That is the target, and it is
the reason this port exists — **not** raw speed (the Python sim is already
sliced and the rendering is already GPU-accelerated).

---

## 2. The execution model

- **Fixed-timestep simulation**, decoupled from render framerate: `FixedUpdate`
  at `SIM_HZ` (10 sim-ticks/sec to start, tunable). The render loop just
  interpolates/presents whatever the sim last wrote.
- **`SimClock`** — continuous seconds (`f64`), replacing `world.turn`. Seasons,
  years and day/night become *derived* time-of-day functions of this clock, not
  turn counters.
- **Systems integrate rates, never "roll a turn."** Production adds
  `rate * dt`; movement adds `speed * dt`; construction adds
  `progress_rate * dt`.
- **Per-entity timers** drive scheduled work: a `Producer` component carries its
  own `interval`/`since`; a `Caravan` carries `remaining_seconds`; a build
  carries `remaining_work`. Nothing is gated on a global day boundary.
- **Determinism stays load-bearing** (the Python project's standing rule): the
  sim is a pure function of `(seed, SimClock, inputs)`; fixed timestep + fixed
  `f64` accumulation keeps it reproducible, and the port's tests fingerprint
  it exactly the way `dev/test_turn_slice.py` / `dev/bench_turn.py --fingerprint`
  did.

---

## 3. Architecture (Bevy plugins)

The Python module layout maps onto a set of Bevy plugins, each ported one at a
time as a **continuous** system:

| Python module | Rust plugin | ports |
|---|---|---|
| `worldgen.py` / `plates.py` / `rivers.py` / `noise.py` | `worldgen` | continents, rainfall, temperature lapse, biomes, rivers, startsites → a `WorldMap` resource |
| (none — new) | `time` | `SimClock`, fixed timestep, season/day/night derived state |
| `worldgen.Settlement/Village` | `settlement` | population, production, needs — per-entity timers |
| `resources.py` | `economy` | resource registry, conversion, storage, spoilage |
| `trade.py` | `trade` | caravans, routes, payments — continuous travel |
| `construction.py` / `expansion.py` | `build` | continuous build progress, claims |
| `battle/`, `commander.py` | `war` | units, movement, combat resolution |
| `governance.py` / `progression.py` | `governance` | government forms, loyalty drift |
| `map_view.py` / `gl_*` | `render` | the 2.5D presentation + HUD |

ECS notes that matter for the redesign:

- **Entities are the settlements/villages/units/caravans**, not the map cells —
  the map is a shared `WorldMap` resource (a grid), exactly the split the
  Python game already settled on.
- **No "phase generator."** The Python `day_steps` generator exists only to
  slice one day's work; in the port each system just runs every fixed tick and
  reads `dt`. The coherence/atomicity problem disappears because there is no
  day to atomize.

---

## 4. Walking skeleton (milestone 0 — this change)

`rust/` — a `shapes_of_war` crate:

- `Cargo.toml` pinned to `bevy = "0.19"`.
- `src/main.rs`:
  - a deterministic procedural **height/biome grid** (no external noise crate
    yet — a seedable sin/hash blend), rendered as colored tiles with a
    height-driven y-offset and z-order (the 2.5D depth cue);
  - a **`SimClock`** advanced at a fixed `SIM_HZ`;
  - a **`Producer`** (a village) that accrues stock on its own timer and logs
    each delivery — scheduled work, not a day roll;
  - a **`Caravan`** that integrates `speed × dt` continuously between two
    points — movement with no "N turns to arrive."

That is the whole milestone: it proves fixed timestep, continuous integration,
and per-entity timers run headless-logic-correct, before any 4X system is
ported.

---

## 5. Roadmap

| milestone | what lands |
|---|---|
| **M0** | walking skeleton (this) |
| **M1** | `worldgen` port — continents, rainfall, temperature lapse, biome blending, rivers — as a continuous `WorldMap` (reuse the Python constants as the spec) |
| **M2** | `time` + `settlement` — seasons derived from the clock; one faction, a settlement that produces/consumes continuously |
| **M3** | `economy` — resource registry, conversion, storage, spoilage |
| **M4** | `trade` + `build` — continuous caravans and construction |
| **M5** | `war` — units, movement, combat |
| **M6** | `governance` — government forms, loyalty drift |
| **M7** | `render` — the real 2.5D presentation + HUD |
| **M8** | save/load (new format; the Python pickle is not carried over) + ship |

Each milestone is independently runnable and revertable; none of them touches
the shipped Python game.

---

## 6. Decisions deferred (do not need answers yet)

- The exact **2.5D projection** (angled orthographic camera vs. true isometric
  tiles vs. height-scaled top-down) — settled when `render` lands.
- **Save format** — new, most likely `serde`/`rkyv`; Python saves are dropped
  rather than migrated.
- Whether the Rust project stays in this repo under `rust/` or is split into
  its own repo later (`git subtree` makes that a cheap, reversible call).
- Final name/title — codename is `shapes_of_war` for now.
