# v0.0.8

Milestone 2 of the continuous-time port: **time + settlement**. The world
starts living — the fixed-timestep `SimClock` replaces the Python game's
sliced "day" entirely.

- **Continuous clock** — sim time is a single f64 advanced at a fixed 10 Hz
  (1 sim-second = 1 day). Season, year and day/night are *derived* from the
  clock, never rolled. The world starts in Spring; a year is 100 days, the
  same pacing as the Python game (~100 real seconds per year).
- **One faction, one town** — "The Reach" founds a town on the map's best
  farmland-with-forest site, and it runs the ported economy continuously:
  - **Production** — 13 crops follow their real growth cycle (only the
    Harvest season yields: Wheat/Rye/Peas/Fodder in Summer, the rest in
    Autumn), while Forestry (Firewood, Logs) runs every day. Rarity-shared
    biome land, climate affinity and fertility weighting all ported
    verbatim from `resources.py`.
  - **Consumption** — the town eats, heats and maintains itself from its own
    stockpile (Food/Firewood/Clothes/Luxury/Timber per-capita needs), with
    starvation/freezing grace periods, severity-scaled population loss, and
    the firewood scrounge that keeps forest-poor land warm.
  - **Growth & prosperity** — population climbs toward the town's ceiling at
    the frontier rate, children mature into a stripped workforce, and
    prosperity eases toward a target shaped by shortages (Food/Firewood/
    Clothes/Timber weights) and luxury fulfillment.
  - **Site selection** — refuses to found the town where it would freeze
    every Winter (the Python fuel-pass lesson: forest-poor settlements lost
    their people until coal and trade arrived — both later milestones).
- **HUD** — clock/season/day-night, demographics, prosperity, stocks,
  per-day production and needs, and living conditions; the Regenerate button
  makes a new world and re-places the town on it.
- **Headless fingerprint tests** — `cargo test`: a fixed seed must produce
  the identical 400-day outcome every run (determinism stays load-bearing),
  plus population-identity and seasonal-harvest invariants. `cargo run
  --example debug_site` prints the town's year-by-year state for a fixed
  seed.
