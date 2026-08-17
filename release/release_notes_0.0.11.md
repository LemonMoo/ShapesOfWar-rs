# v0.0.11

Milestone 5 begins: **war** — the first slice (M5a) lands the unit layer of
the battle system as pure, headless, continuous-time code.

## What's new (headless)

- **The unit registry** — all ten unit archetypes and the five species
  commander profiles, ported from the Python battle system's
  `unit_types.py` as pure data. Add a type here and it is immediately
  usable by any army.
- **Continuous unit movement** — units integrate `speed × dt` in cell
  space, steered by the same seek + separation + avoidance as the Python
  `movement.py`: no pathfinding, capped deflection, a spatial grid for
  per-tick neighbour lookup.
- **Mustering** — the levy pipeline: levy = adults × mobilization rate,
  armed/militia split, shield and cavalry bonuses, floor and ceiling.
- **The battle sub-clock** — a battle is an event, not a season:
  1 sim-second (1 day) = 60 battle-seconds, so a 60-second battle takes
  one in-game day. Positions are grid cells, not canvas pixels.

## Honest note: nothing looks different in the game yet

M5a is the *foundation* slice of the war milestone. The `war` module is
exported from the library and fully tested (12 new headless tests, all
passing), but it is **not yet wired into the running sim** — no battle is
triggered, no units appear on the map, and the HUD is unchanged. This
release exists to validate the release/launcher pipeline for the war
milestone; the playable payoff comes with M5b (combat resolution) and the
sim integration that follows.

## Tests

39 headless tests pass, including the 12 new M5a ones: the unit registry
against the Python constants, steering invariants (deflection cap,
separation, arrival), the mustering math (floor, ceiling, militia split,
bonuses), and a full-battle determinism fingerprint (two deployed armies
ticking for a day, bit-identical on a fixed seed).
