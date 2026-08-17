# v0.0.9

Milestone 3 of the continuous-time port: **economy** — resource registry,
conversion, storage, spoilage. M2's town produced and ate crops; M3 gives the
goods model the Python game actually has, as continuous systems.

- **Resource registry** — the full `RESOURCES` table from `resources.py`
  (60 resources): category, tier, per-resource `spoil_rate`, `bulk` (storage
  space per unit), storage pool, `edible` and `luxury` flags — the
  authoritative spec M4+ milestones read, exactly the way M2 carried the
  tier/rarity tables ahead of their use. Value now prices off the registry's
  tiers (`resource_value`), so a good's worth is one number everywhere.
- **Conversion** — the `RECIPES` table ported and run continuously at the
  settlement: Wheat → Flour → Bread, Logs → Planks, Cotton → Cloth →
  Clothes, Paper, and — only after a full year, at a deliberate trickle — the
  luxuries (Wine, Beer, Furniture, Fine Clothes). 1:1 ratios,
  `CONVERSION_RATE_CAP` 30/day, `LUXURY_CONVERSION_RATE_CAP` 2/day,
  scarcest-input binding, first-available alternative. Clothes and Luxury —
  permanent prosperity shortfalls in M2 — are now actually producible.
- **Storage** — the four typed town pools (granary 840 / warehouse 750 /
  vault 150 / barn 200) with bulk-weighted occupancy, and the
  `storage_throttle`: production tapers as its pool fills (85% → 15% floor),
  so a full barn throttles the harvest instead of silently destroying the
  overage. Firewood is exempt — a winter fuel hoard must never be squeezed
  out of a full pantry.
- **Spoilage** — every resource decays at its registry rate (Wheat 3%/day,
  Potatoes 6%, Bread 35%, Fish 35%, Logs never), and a pool packed past
  capacity decays its overage on top of that (`OVERFLOW_SPOILAGE_MULTIPLIER`
  etc.), tapering as the overflow shrinks.
- **Faithful consumption pools** — Food now pools the registry-derived
  `_FOOD_SOURCES` (edible crops + Food Products + Fish + the edible
  subterranean foods — no Cotton/Fodder, which are fibre and animal feed,
  not food), Timber pools Planks + Logs, and Luxury pools every Luxury Good.
- **Determinism, made structural** — the sim's determinism guarantee is
  load-bearing, and Rust's std `HashMap` iterates in a random per-map order.
  M2 happened to survive (its order-sensitive maps had ≤ 2 keys, and `a+b ==
  b+a`); M3's multi-key pools would not. Every cross-resource float
  accumulation now iterates a fixed order — the `RESOURCES` table for
  `pool_stock`, sorted output for `production_rates`, sorted keys for the
  prosperity shortage product — so a fixed seed yields the identical outcome
  on every run, verified by the extended fingerprint test.
- **HUD & debug** — the HUD gains Storage (per-pool `used/cap`) and
  Converting (today's recipe rates) lines; `debug_site` prints planks/
  clothes/wine, storage fill and the live conversion plan each year.
- **Tests** — 11 new headless tests: registry completeness/tier/pool
  consistency, food-pool/registry reconciliation, recipe-name integrity,
  the Wheat→Flour→Bread chain, the year-long luxury gate + trickle,
  first-alternative-wins (and the Paper mill catching the Cotton the loom
  passes over), per-resource spoilage, pool overflow decay, Gold's
  overflow immunity, the throttle taper, and a continuous-sim fingerprint.
  `cargo test` — 13 pass.
