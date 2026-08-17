# v0.0.10

Milestone 4 of the continuous-time port: **trade + build** — a second realm
arrives, the two capitals build a route toward each other, and continuous
caravans carry goods between them for gold. The `trade` half of the plan's
M4 is fully alive; the `build` half — the kingdom treasury and the storage
buildings — is in, with its AI gated exactly like the Python game.

## The second realm & trade

- **A neighbour** — the world now has two factions, each with its own town.
  The AI capital is placed far from yours (≥45% of the map's width), so
  trade exists to bridge real distance.
- **Routes** — the capitals propose a route on day one. A **land route**
  is built over time by both realms working toward each other at 6 cells/day
  (`TRADE_ROUTE_CELLS_PER_TURN`); when the two landmasses aren't connected,
  a **sea route** opens immediately (the Python's "river/sea routes open
  immediately").
- **Continuous caravans** — a `TradeCaravan` travels on its own clock
  (`progress += dt`, no "N turns to arrive"), sized by the seller's surplus
  and the buyer's spendable gold, floored at `MIN_TRADE_QUANTITY`. Payment is
  two-phase exactly like the Python: goods are delivered at the buyer, and
  **gold is credited to the seller only when the caravan returns home** —
  and the crown skims `TRADE_TAX_RATE` (10%) of it into the seller's
  treasury. Pricing is the Python `unit_price`: base tier value, discounted
  by the seller's surplus, marked up by the buyer's scarcity, with the
  safety reserve (`needs × 8` for food/needs, 10% of storage cap for
  durables) never sold away.
- **The currency loop** — Gold Ore is now mined from mountain/highland cells
  and the Mint (already in the M3 recipe registry) strikes it into coin —
  the Python's "gold only enters via mining" rule, and the missing piece
  that keeps trade liquidity alive. Each realm starts with its share of
  `STARTING_GOLD_PER_FACTION` (4000) in its settlements.

## Build: the kingdom treasury & storage buildings

- **The treasury** — every realm keeps a central kingdom treasury (seeded
  with `STARTING_TREASURY_PER_FACTION` 2000). An **income tax** drains each
  settlement's held gold at its rolled `tax_income` rate (towns 2–4/day),
  and the transaction tax above skims trade too. Taxes redistribute, never
  mint. The treasury alone pays the Gold line of construction.
- **Storage buildings** — granary / warehouse / vault, with the exact Python
  cost and build-time tables (tier 1 granary: Logs 300 + Stone 100 + Gold
  150, 15 days; up to tier 3/3/2). Cost is paid **up front** (Gold from the
  treasury, goods drawn from the realm's stockpiles largest-first), then the
  building accrues `progress += dt` at one day per day — and on completion
  the pool capacity grows by the tier's `STORAGE_TIER_BONUS` (a tier-1
  granary takes the town's granary pool 840 → 2040).
- **The storage AI** — builds when a pool is under pressure (≥80% full),
  worst-pressure pool first, one project per settlement.

## Honest notes on the demo

- **Construction is gated on materials, exactly like the Python.** The
  Python's own notes measure storage buildings as "eligible 308 times over
  30 turns and affording it zero times, most often for want of Stone". The
  Rust demo has one small town per realm (no villages or mining camps yet),
  so a town whose catchment lacks forest/stone may simply never afford a
  granary — hit **Regenerate** for a world where it can (the HUD shows the
  Building line the moment a project starts). The machinery, treasury
  funding and AI are all in; the villages/camps that make construction fire
  routinely arrive with later milestones.
- **Some pairs never connect** — a genuinely landlocked pair of capitals
  (neither can reach open water) has no route and no trade, the Python's
  own "these realms are simply not connected" outcome. Rare, and the sea
  fallback covers most maps.
- No war yet, so no caravan loss rolls and no blockades — both realms are
  neutral and in contact from the start.

## Tests

27 headless tests pass, including four new M4 ones: the seeded worldgen
reserve (gold share + larder, never over capacity), the far-apart capitals,
the sea-route fallback, and a **full-M4-world determinism fingerprint**
(both towns, treasuries, route and caravans over 400 days, bit-identical on
a fixed seed).
