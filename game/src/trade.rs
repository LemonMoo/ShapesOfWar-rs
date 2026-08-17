//! `trade` — continuous caravans and construction of routes (port milestone
//! M4, the foreign tier of `app/world/trade.py`).
//!
//! M2/M3 built one faction and one town; M4 adds a second AI faction and its
//! own capital, and this module makes the two realms trade — continuously.
//! Ported pieces (constants carry the Python value + source symbol):
//!
//! - **routes** — `_land_capital_path`: a land Dijkstra between the two
//!   capitals, built over time by a `TradeRouteProject` at
//!   `TRADE_ROUTE_CELLS_PER_TURN` cells/day (both ends working toward the
//!   middle), and only then usable by caravans.
//! - **pricing** — `unit_price`: base tier value, discounted by the seller's
//!   surplus and marked up by the buyer's scarcity (`_safety_reserve` —
//!   needs×`SAFETY_RESERVE_TURNS` for food/needs, 10% of storage cap for
//!   durables). Pure formula, no market hall.
//! - **caravans** — `TradeCaravan` with a continuous `progress += dt`
//!   clock (turns are days), a two-phase payment (goods delivered at the
//!   buyer on arrival, **gold credited to the seller only when the caravan
//!   returns home**), and the transaction tax (`TRADE_TAX_RATE`) skimmed
//!   into the seller's treasury on credit.
//! - **the trade AI** — `run_trade_ai`: greedy first-match per faction,
//!   capped at `MAX_ACTIVE_TRADES_PER_FACTION`, deals sized by the buyer's
//!   spendable gold (`GOLD_TRADE_RESERVE` floor), floored at
//!   `MIN_TRADE_QUANTITY`, Gold-only payment (foreign trade never barters).
//!
//! M4 simplifications, each deliberate (documented in the module docs of
//! `settlement` too): both realms start in contact and neutral (no diplomacy
//! milestone yet), so the route is proposed on day one; there is only one
//! pair of nodes, so one route and no route-choice AI; and the caravan loss
//! check (`LAND_RISK_PER_TURN`) waits for M5 war, when hostile cells exist.

use std::collections::BinaryHeap;

use crate::economy::{self, RESOURCES};
use crate::settlement::{needs, Settlement};
use crate::time::{Season, SimClock};
use crate::worldgen::WorldMap;

// --- ported constants (trade.py) --------------------------------------------
pub const MIN_TRADE_QUANTITY: f64 = 20.0;
pub const SAFETY_RESERVE_TURNS: f64 = 8.0; // food: never sell below N days of upkeep
pub const NON_FOOD_RESERVE_FRACTION: f64 = 0.1; // non-food: keep 10% of storage cap
pub const MAX_ACTIVE_TRADES_PER_FACTION: usize = 3;
pub const CELLS_PER_TURN: f64 = 15.0; // land caravan pace
pub const MIN_TRANSIT_TURNS: f64 = 5.0;
pub const MAX_TRANSIT_TURNS: f64 = 20.0;
/// How many route cells a `TradeRouteProject` finishes per day (both ends
/// working inward) — `TRADE_ROUTE_CELLS_PER_TURN`.
pub const TRADE_ROUTE_CELLS_PER_TURN: f64 = 6.0;
/// Gold a settlement keeps for its own spending (claims, construction) and
/// never offers up in commerce — `GOLD_TRADE_RESERVE`.
pub const GOLD_TRADE_RESERVE: f64 = 200.0;
/// Turns before a pair that failed to find a path tries again —
/// `TRADE_ROUTE_DECLINE_COOLDOWN_TURNS`.
pub const TRADE_ROUTE_DECLINE_COOLDOWN_TURNS: f64 = 10.0;

// --- route finding (land only, M4) ------------------------------------------

/// A route under construction or open, and how it was found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PathKind {
    /// A land path the capitals build toward each other over time
    /// (`_land_capital_path` → `TradeRouteProject`).
    Land,
    /// An ocean path that opens immediately (`_capital_sea_path` — "sea
    /// routes open immediately" in the Python).
    Sea,
}

/// The route proposal computed once at worldgen, held until the route AI
/// claims it on day one.
pub struct PendingPath {
    pub kind: PathKind,
    pub cells: Vec<(i32, i32)>,
}

/// A completed trade route: the cell list between the two capitals, in
/// seller→buyer order (the Python `{"kind": "land", "cells": [...]}`).
#[derive(Clone, Debug)]
pub struct Route {
    pub cells: Vec<(i32, i32)>,
}

/// A land route under construction: both realms build toward each other at
/// `TRADE_ROUTE_CELLS_PER_TURN` cells/day; `progress` counts built cells.
#[derive(Clone, Debug)]
pub struct TradeRouteProject {
    pub path: Vec<(i32, i32)>,
    pub progress: f64,
}

impl TradeRouteProject {
    pub fn total_cells(&self) -> f64 {
        (self.path.len() - 1).max(0) as f64
    }
}

/// Shared Dijkstra between two cells over cells `passable` says are
/// traversable (wrap-aware in x, 8 neighbours, cost 1). Returns the path
/// start→goal. Used for land paths (passable = land) and sea paths
/// (passable = open ocean).
fn dijkstra(
    map: &WorldMap,
    a: (i32, i32),
    b: (i32, i32),
    passable: impl Fn(usize) -> bool,
) -> Option<Vec<(i32, i32)>> {
    let (w, h) = (map.w, map.h);
    let n = (w * h) as usize;
    let start = (a.1 * w + a.0) as usize;
    let goal = (b.1 * w + b.0) as usize;
    let mut dist = vec![f64::INFINITY; n];
    let mut prev: Vec<Option<usize>> = vec![None; n];
    dist[start] = 0.0;
    // Distances are non-negative, so `to_bits` preserves order and gives a
    // total order for the heap (f64 itself is not Ord).
    let mut heap = BinaryHeap::new();
    heap.push(std::cmp::Reverse((0.0f64.to_bits(), start)));
    let mut found = false;
    while let Some(std::cmp::Reverse((dbits, i))) = heap.pop() {
        if i == goal {
            found = true;
            break;
        }
        let d = f64::from_bits(dbits);
        if d > dist[i] {
            continue;
        }
        let (x, y) = (i as i32 % w, i as i32 / w);
        for (dx, dy) in [
            (1, 0), (-1, 0), (0, 1), (0, -1),
            (1, 1), (1, -1), (-1, 1), (-1, -1),
        ] {
            let nx = (x + dx).rem_euclid(w);
            let ny = y + dy;
            if ny < 0 || ny >= h {
                continue;
            }
            let ni = (ny * w + nx) as usize;
            if !passable(ni) {
                continue;
            }
            let nd = d + 1.0;
            if nd < dist[ni] {
                dist[ni] = nd;
                prev[ni] = Some(i);
                heap.push(std::cmp::Reverse((nd.to_bits(), ni)));
            }
        }
    }
    if !found {
        return None;
    }
    let mut path = Vec::new();
    let mut cur = Some(goal);
    while let Some(i) = cur {
        let (x, y) = (i as i32 % w, i as i32 / w);
        path.push((x, y));
        cur = prev[i];
    }
    path.reverse();
    Some(path)
}

/// Dijkstra over land cells between `a` and `b` (wrap-aware in x, 8
/// neighbours, cost 1 per cell) — the port's `_land_capital_path` /
/// `_path_between`. `None` when the two landmasses are not connected by
/// land (a sea route is tried next).
pub fn land_path(map: &WorldMap, a: (i32, i32), b: (i32, i32)) -> Option<Vec<(i32, i32)>> {
    dijkstra(map, a, b, |i| map.land.v[i] && !map.lake.v[i])
}

/// BFS from `start` over land to the nearest open-ocean cell; returns the
/// walk (including the ocean cell at the end). `None` only if the capital
/// is on a landmass with no coast at all — an inland sea with no outlet.
fn nearest_ocean(map: &WorldMap, start: (i32, i32)) -> Option<(Vec<(i32, i32)>, (i32, i32))> {
    let (w, h) = (map.w, map.h);
    let n = (w * h) as usize;
    let start_i = (start.1 * w + start.0) as usize;
    let mut prev: Vec<Option<usize>> = vec![None; n];
    let mut seen = vec![false; n];
    seen[start_i] = true;
    let mut queue = std::collections::VecDeque::from([start_i]);
    let mut ocean_i = None;
    while let Some(i) = queue.pop_front() {
        if !map.land.v[i] {
            ocean_i = Some(i);
            break;
        }
        let (x, y) = (i as i32 % w, i as i32 / w);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let nx = (x + dx).rem_euclid(w);
            let ny = y + dy;
            if ny < 0 || ny >= h {
                continue;
            }
            let ni = (ny * w + nx) as usize;
            if !seen[ni] {
                seen[ni] = true;
                prev[ni] = Some(i);
                queue.push_back(ni);
            }
        }
    }
    let ocean_i = ocean_i?;
    let ocean_pos = (ocean_i as i32 % w, ocean_i as i32 / w);
    let mut trail = Vec::new();
    let mut cur = Some(ocean_i);
    while let Some(i) = cur {
        let (x, y) = (i as i32 % w, i as i32 / w);
        trail.push((x, y));
        cur = prev[i];
    }
    trail.reverse();
    Some((trail, ocean_pos))
}

/// A sea route between the two capitals (`_capital_sea_path` — both realms
/// coastal enough to reach open water): each capital walks to its nearest
/// ocean cell, then the ocean is crossed directly. Opens immediately (the
/// Python's "river/sea routes open immediately"); `None` if either capital
/// cannot reach open water at all.
pub fn sea_path(map: &WorldMap, a: (i32, i32), b: (i32, i32)) -> Option<Vec<(i32, i32)>> {
    let (trail_a, shore_a) = nearest_ocean(map, a)?;
    let (trail_b, shore_b) = nearest_ocean(map, b)?;
    let crossing = dijkstra(map, shore_a, shore_b, |i| !map.land.v[i])?;
    let mut path = trail_a;
    path.extend(crossing.iter().skip(1).copied());
    path.extend(trail_b.iter().rev().skip(1).copied());
    Some(path)
}

/// Transit time in days for a caravan over `n_cells` (`base_turns`, the
/// Python `max(floor, min(MAX_TRANSIT_TURNS, round(len/cells_per_turn)))`;
/// M4 has no allies, so the speed multiplier is 1.0).
pub fn transit_turns(n_cells: usize) -> f64 {
    let base = (n_cells as f64 / CELLS_PER_TURN).round();
    let base = base.min(MAX_TRANSIT_TURNS).max(MIN_TRANSIT_TURNS);
    base.max(MIN_TRANSIT_TURNS)
}

// --- pricing (unit_price, _safety_reserve) -----------------------------------

/// Gold a settlement is willing to spend in commerce — its held Gold above
/// the `GOLD_TRADE_RESERVE` floor (`_spendable_gold`).
pub fn spendable_gold(s: &Settlement) -> f64 {
    let held = s.resources.get("Gold").copied().unwrap_or(0.0);
    (held - GOLD_TRADE_RESERVE).max(0.0)
}

/// The safety reserve for one resource (`_safety_reserve`): a need is never
/// sold below `needs × SAFETY_RESERVE_TURNS`; a durable is kept at 10% of
/// storage cap.
pub fn safety_reserve(s: &Settlement, res: &str, season: Season) -> f64 {
    let n = needs(s, season);
    let pooled = |key: &str| n.get(key).copied().unwrap_or(0.0) * SAFETY_RESERVE_TURNS;
    if economy::FOOD_SOURCES.contains(&res) {
        pooled("Food")
    } else if res == "Firewood" {
        pooled("Firewood")
    } else if res == "Clothes" {
        pooled("Clothes")
    } else if economy::LUXURY_GOODS.contains(&res) {
        pooled("Luxury")
    } else if economy::TIMBER_SOURCES.contains(&res) {
        pooled("Timber")
    } else {
        economy::settlement_storage_capacity(&s.storage) * NON_FOOD_RESERVE_FRACTION
    }
}

/// Surplus available for export (`sellable_surplus` — stock minus the
/// safety reserve), floored at 0.
pub fn sellable_surplus(s: &Settlement, res: &str, season: Season) -> f64 {
    let stock = s.resources.get(res).copied().unwrap_or(0.0);
    (stock - safety_reserve(s, res, season)).max(0.0)
}

/// Buyer scarcity 0..1 (`buyer_need` settlement branch): 0 = full,
/// 1 = desperate, against the buyer's total storage capacity.
pub fn buyer_need(buyer: &Settlement, res: &str) -> f64 {
    let cap = economy::settlement_storage_capacity(&buyer.storage);
    if cap <= 0.0 {
        return 0.0;
    }
    let stock = buyer.resources.get(res).copied().unwrap_or(0.0);
    (1.0 - stock / cap).max(0.0)
}

/// `unit_price` (settlement branch): base tier value, discounted by the
/// seller's surplus (`seller_factor`), marked up by the buyer's scarcity
/// (`buyer_factor`), rounded to 2 decimals. No ally discount in M4 — both
/// realms are neutral.
pub fn unit_price(seller: &Settlement, buyer: &Settlement, res: &str, season: Season) -> f64 {
    let tier = economy::spec(res).map(|s| s.tier as usize).unwrap_or(3);
    let base = economy::BASE_VALUE_BY_TIER[tier];
    let surplus = sellable_surplus(seller, res, season);
    let cap = economy::settlement_storage_capacity(&seller.storage);
    let surplus_ratio = (surplus / (cap * 0.1 + 1.0)).min(2.0);
    let seller_factor = (1.2 - 0.4 * surplus_ratio).max(0.6);
    let buyer_factor = (0.7 + 1.8 * buyer_need(buyer, res)).min(2.5);
    let price = base * seller_factor * buyer_factor;
    (price * 100.0).round() / 100.0
}

// --- the caravan -------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Leg {
    Outbound,
    Return,
}

/// A goods caravan between two capitals (`TradeCaravan`). Travel is a
/// continuous clock: `progress` accrues `dt` per day, and a leg completes
/// when it reaches `turns_total`. The load is quantity-based — bounded by
/// the seller's surplus, the buyer's spendable gold and `MIN_TRADE_QUANTITY`,
/// never by a cargo slot.
#[derive(Clone, Debug)]
pub struct TradeCaravan {
    pub id: u32,
    pub seller_idx: usize,
    pub buyer_idx: usize,
    pub resource: String,
    pub quantity: f64,
    pub unit_price: f64,
    pub total_price: f64,
    pub path: Vec<(i32, i32)>,
    pub progress: f64,
    pub turns_total: f64,
    pub leg: Leg,
    /// Payment collected at the buyer; empty until outbound delivery, and
    /// credited to the seller only when the return leg completes.
    pub payment: Vec<(String, f64)>,
}

// --- world state -------------------------------------------------------------

/// The world's trade state: one route (or one under construction), the
/// pending route proposal computed at worldgen, and the caravans at sea
/// (well, on land).
#[derive(Resource)]
pub struct TradeState {
    pub routes: Vec<Route>,
    pub route_projects: Vec<TradeRouteProject>,
    pub caravans: Vec<TradeCaravan>,
    /// The path between the two capitals, found once at worldgen; the route
    /// AI claims it on day one (both realms start in contact). Land paths
    /// become a construction project; sea paths open immediately.
    pending_path: Option<PendingPath>,
    pub next_caravan_id: u32,
}

impl TradeState {
    /// Build the trade state for a two-faction world: find the path between
    /// the two capitals (once — the map is static) and hold it as the route
    /// AI's pending proposal. A land path is preferred; a sea route is the
    /// fallback when the capitals sit on different landmasses.
    pub fn new(map: &WorldMap, a: &Settlement, b: &Settlement) -> TradeState {
        let pending_path = land_path(map, a.pos, b.pos)
            .map(|cells| PendingPath { kind: PathKind::Land, cells })
            .or_else(|| {
                sea_path(map, a.pos, b.pos).map(|cells| PendingPath { kind: PathKind::Sea, cells })
            });
        TradeState {
            routes: Vec::new(),
            route_projects: Vec::new(),
            caravans: Vec::new(),
            pending_path,
            next_caravan_id: 0,
        }
    }
}

// --- the daily flow (advance_caravans / run_trade_ai, continuous) -----------

/// Claim the pending route (the Python route AI, one pair): with both
/// realms in contact and neutral from the start, the pending path becomes a
/// `TradeRouteProject` (land — the capitals build toward each other) or an
/// open `Route` (sea — "sea routes open immediately").
pub fn run_route_ai(state: &mut TradeState) {
    if !state.routes.is_empty() || !state.route_projects.is_empty() {
        return;
    }
    if let Some(p) = state.pending_path.take() {
        if p.cells.len() >= 2 {
            match p.kind {
                PathKind::Land => {
                    println!(
                        "[trade] both realms begin building a land route ({} cells)",
                        p.cells.len() - 1
                    );
                    state.route_projects.push(TradeRouteProject { path: p.cells, progress: 0.0 });
                }
                PathKind::Sea => {
                    println!(
                        "[trade] a sea route between the capitals opens immediately ({} cells)",
                        p.cells.len() - 1
                    );
                    state.routes.push(Route { cells: p.cells });
                }
            }
        }
    }
}

/// Advance route construction: `TRADE_ROUTE_CELLS_PER_TURN` cells/day; a
/// finished project becomes a usable `Route`.
pub fn advance_route_projects(state: &mut TradeState, dt: f64) {
    if dt <= 0.0 {
        return;
    }
    let mut done: Vec<usize> = Vec::new();
    for (i, p) in state.route_projects.iter_mut().enumerate() {
        p.progress += TRADE_ROUTE_CELLS_PER_TURN * dt;
        if p.progress >= p.total_cells() {
            done.push(i);
        }
    }
    for (k, &i) in done.iter().enumerate() {
        let p = state.route_projects.remove(i - k);
        println!("[trade] the land route is complete — caravans may travel");
        state.routes.push(Route { cells: p.path });
    }
}

/// Advance every caravan one tick. Outbound arrival delivers the goods to
/// the buyer (never rejected at the door — overflow is handled by spoilage
/// later) and collects a Gold-only payment; return arrival credits the
/// seller (minus the transaction tax) and retires the caravan. The M4
/// world has no enemies, so the `LAND_RISK_PER_TURN` loss roll waits for
/// M5 war.
pub fn advance_caravans<'a>(
    state: &mut TradeState,
    settlements: impl IntoIterator<Item = &'a mut Settlement>,
    treasury: &mut crate::build::Treasury,
    dt: f64,
) {
    if dt <= 0.0 {
        return;
    }
    let mut settlements: Vec<&mut Settlement> = settlements.into_iter().collect();
    let mut finished: Vec<u32> = Vec::new();
    for c in state.caravans.iter_mut() {
        c.progress += dt;
        if c.progress < c.turns_total {
            continue;
        }
        match c.leg {
            Leg::Outbound => {
                let buyer = settlements.iter_mut().find(|s| s.id as usize == c.buyer_idx).unwrap();
                *buyer.resources.entry(c.resource.clone()).or_insert(0.0) += c.quantity;
                // Gold-only payment (foreign trade never barters), capped at
                // the buyer's spendable gold.
                let take = spendable_gold(buyer).min(c.total_price);
                if take > 0.0 {
                    *buyer.resources.get_mut("Gold").unwrap() -= take;
                    c.payment = vec![("Gold".to_string(), take)];
                }
                c.leg = Leg::Return;
                c.progress = 0.0;
                println!(
                    "[trade] caravan {} delivered {:.0} {} to {} for {:.0} gold; heading home",
                    c.id,
                    c.quantity,
                    c.resource,
                    settlements.iter().find(|s| s.id as usize == c.buyer_idx).unwrap().name,
                    take
                );
            }
            Leg::Return => {
                let seller = settlements.iter_mut().find(|s| s.id as usize == c.seller_idx).unwrap();
                for (r, qty) in &c.payment {
                    if r == "Gold" {
                        let tax = qty * crate::build::TRADE_TAX_RATE;
                        let net = qty - tax;
                        *seller.resources.entry("Gold".to_string()).or_insert(0.0) += net;
                        treasury.credit(seller.faction_idx, tax);
                    } else {
                        *seller.resources.entry(r.clone()).or_insert(0.0) += qty;
                    }
                }
                println!(
                    "[trade] caravan {} returned — {} paid {:.0} gold ({} to the treasury)",
                    c.id,
                    settlements.iter().find(|s| s.id as usize == c.seller_idx).unwrap().name,
                    c.payment.iter().map(|(_, q)| q).sum::<f64>() * (1.0 - crate::build::TRADE_TAX_RATE),
                    c.payment.iter().map(|(_, q)| q).sum::<f64>() * crate::build::TRADE_TAX_RATE,
                );
                finished.push(c.id);
            }
        }
    }
    state.caravans.retain(|c| !finished.contains(&c.id));
}

/// The trade AI (`run_trade_ai`, greedy first-match): each faction, capped
/// at `MAX_ACTIVE_TRADES_PER_FACTION` outbound caravans, scans the registry
/// (fixed table order — deterministic) for the first good it has surplus of
/// that the other realm actually needs, sizes the deal by the buyer's
/// spendable gold, and dispatches. First match per faction wins.
pub fn run_trade_ai<'a>(
    state: &mut TradeState,
    settlements: impl IntoIterator<Item = &'a mut Settlement>,
    season: Season,
) {
    if state.routes.is_empty() {
        return; // no road yet — nothing can travel
    }
    let mut settlements: Vec<&mut Settlement> = settlements.into_iter().collect();
    let route = state.routes[0].clone();
    for seller_idx in 0..2usize {
        let active = state
            .caravans
            .iter()
            .filter(|c| c.seller_idx == seller_idx)
            .count();
        if active >= MAX_ACTIVE_TRADES_PER_FACTION {
            continue;
        }
        let buyer_idx = 1 - seller_idx;
        // Disjoint mutable borrows of the two capitals (ids are 0 and 1).
        let (first, second) = settlements.split_at_mut(1);
        let (seller, buyer): (&mut Settlement, &mut Settlement) = if seller_idx == 0 {
            (&mut first[0], &mut second[0])
        } else {
            (&mut second[0], &mut first[0])
        };
        for spec in RESOURCES {
            if spec.name == "Gold" {
                continue; // gold is the currency, never the cargo
            }
            let surplus = sellable_surplus(seller, spec.name, season);
            if surplus < MIN_TRADE_QUANTITY {
                continue;
            }
            if buyer_need(buyer, spec.name) <= 0.0 {
                continue;
            }
            let price = unit_price(seller, buyer, spec.name, season);
            if price <= 0.0 {
                continue;
            }
            let power = spendable_gold(buyer);
            let qty = surplus.min((power / price).floor());
            if qty < MIN_TRADE_QUANTITY {
                continue;
            }
            // Dispatch: goods leave the seller now; the buyer pays on arrival.
            *seller.resources.get_mut(spec.name).unwrap() -= qty;
            let total_price = (qty * price * 100.0).round() / 100.0;
            let turns_total = transit_turns(route.cells.len());
            let id = state.next_caravan_id;
            state.next_caravan_id += 1;
            state.caravans.push(TradeCaravan {
                id,
                seller_idx,
                buyer_idx,
                resource: spec.name.to_string(),
                quantity: qty,
                unit_price: price,
                total_price,
                path: route.cells.clone(),
                progress: 0.0,
                turns_total,
                leg: Leg::Outbound,
                payment: Vec::new(),
            });
            println!(
                "[trade] {} dispatches {:.0} {} to {} at {:.2} each ({:.0} gold, {} days)",
                seller.name,
                qty,
                spec.name,
                buyer.name,
                price,
                total_price,
                turns_total
            );
            break; // first match per faction
        }
    }
}

/// One full world-level trade tick: propose routes, build them, move
/// caravans, dispatch new ones. Pure and deterministic — the Bevy system,
/// headless tests and examples all call this. Settlements may arrive in any
/// order; they are sorted by id here (the sim's determinism contract). Runs
/// AFTER the settlements' own `sim_tick` (production first, then trade) and
/// BEFORE `build::sim_tick` (taxation after trade, so it taxes this turn's
/// proceeds).
pub fn sim_tick<'a>(
    state: &mut TradeState,
    settlements: impl IntoIterator<Item = &'a mut Settlement>,
    treasury: &mut crate::build::Treasury,
    clock: &SimClock,
    dt: f64,
) {
    let mut settlements: Vec<&mut Settlement> = settlements.into_iter().collect();
    settlements.sort_by(|a, b| a.id.cmp(&b.id));
    run_route_ai(state);
    advance_route_projects(state, dt);
    advance_caravans(state, settlements.iter_mut().map(|s| &mut **s), treasury, dt);
    run_trade_ai(state, settlements.iter_mut().map(|s| &mut **s), clock.season());
}

// --- Bevy plugin -------------------------------------------------------------

use bevy::prelude::*;

/// The trade plugin: runs the world-level trade tick each fixed step, after
/// the settlements' own economy tick (`settlement::sim_system`).
pub struct TradePlugin;

impl Plugin for TradePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (trade_system).after(crate::settlement::sim_system),
        );
    }
}

pub(crate) fn trade_system(
    mut state: ResMut<TradeState>,
    mut q: Query<&mut Settlement>,
    mut treasury: ResMut<crate::build::Treasury>,
    clock: Res<SimClock>,
    time: Res<Time<Fixed>>,
) {
    let mut v: Vec<bevy::ecs::change_detection::Mut<Settlement>> = q.iter_mut().collect();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    let refs: Vec<&mut Settlement> = v.iter_mut().map(|s| &mut **s).collect();
    sim_tick(&mut state, refs, &mut treasury, &clock, time.delta_secs_f64());
}

// --- tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::Treasury;
    use crate::worldgen;

    fn world_and_towns() -> (WorldMap, Vec<Settlement>) {
        let map = worldgen::generate(256, 160, 2024);
        let a = Settlement::spawn(&map, 2024);
        let b = Settlement::spawn_at(&map, 2024, 1, 1, Some(a.pos));
        (map, vec![a, b])
    }

    #[test]
    fn land_path_connects_the_capitals_over_land() {
        let (map, towns) = world_and_towns();
        let path = land_path(&map, towns[0].pos, towns[1].pos);
        assert!(path.is_some(), "the two capitals must be land-connected");
        let path = path.unwrap();
        assert_eq!(path.first(), Some(&towns[0].pos));
        assert_eq!(path.last(), Some(&towns[1].pos));
        // Every cell on the path is land.
        for &(x, y) in &path {
            let i = (y * map.w + x) as usize;
            assert!(map.land.v[i] && !map.lake.v[i], "path crosses water at ({x},{y})");
        }
    }

    #[test]
    fn every_capital_pair_gets_a_route_land_or_sea() {
        // Across several seeds, either the capitals are land-connected (a
        // route project) or open water connects them (an immediate sea
        // route). The far-site placement may put them on different
        // landmasses; the sea fallback is what keeps trade alive there.
        // A genuinely landlocked pair (neither capital can reach open
        // water) is the Python's own "these realms are simply not
        // connected" outcome — rare, not the rule.
        let mut land = 0;
        let mut sea = 0;
        let mut none = 0;
        for seed in [2024u64, 777, 4242, 555, 999, 31337] {
            let map = worldgen::generate(256, 160, seed);
            let a = Settlement::spawn(&map, seed);
            let b = Settlement::spawn_at(&map, seed, 1, 1, Some(a.pos));
            if let Some(path) = land_path(&map, a.pos, b.pos) {
                assert!(path.len() >= 2);
                assert_eq!(path.first(), Some(&a.pos));
                assert_eq!(path.last(), Some(&b.pos));
                land += 1;
                continue;
            }
            if let Some(path) = sea_path(&map, a.pos, b.pos) {
                sea += 1;
                assert_eq!(path.first(), Some(&a.pos));
                assert_eq!(path.last(), Some(&b.pos));
                // Adjacent cells only (8-neighbour steps, wrap in x).
                assert!(path.windows(2).all(|w| {
                    let dx = (w[1].0 - w[0].0).abs().min(map.w - (w[1].0 - w[0].0).abs());
                    dx + (w[1].1 - w[0].1).abs() <= 2
                }));
                continue;
            }
            none += 1;
        }
        assert!(sea >= 1, "the seed set must exercise the sea fallback");
        assert!(none <= 2, "landlocked pairs must be the rare exception: {none}");
    }

    #[test]
    fn full_m4_world_is_deterministic() {
        use crate::build::{self, BuildState, Treasury};
        use crate::settlement::seed_initial_stockpiles;
        let run = || {
            let map = worldgen::generate(256, 160, 2024);
            let mut towns = vec![Settlement::spawn(&map, 2024)];
            towns.push(Settlement::spawn_at(&map, 2024, 1, 1, Some(towns[0].pos)));
            seed_initial_stockpiles(&mut towns);
            let mut trade_state = TradeState::new(&map, &towns[0], &towns[1]);
            let mut treasury = Treasury::new(2);
            let mut build_state = BuildState::default();
            let mut seconds = 0.0;
            while seconds < 400.0 {
                for s in towns.iter_mut() {
                    crate::settlement::sim_tick(s, &SimClock { seconds }, 1.0);
                }
                sim_tick(&mut trade_state, towns.iter_mut(), &mut treasury, &SimClock { seconds }, 1.0);
                build::sim_tick(&mut build_state, towns.iter_mut(), &mut treasury, 1.0);
                seconds += 1.0;
            }
            // The whole M4 world: both towns' stocks + storage tiers,
            // treasuries, the route, and the caravan state.
            let mut fp: Vec<(f64, f64, f64, u8, u8, u8, Vec<(String, f64)>)> = towns
                .iter()
                .map(|s| {
                    let mut v: Vec<(String, f64)> = s.resources.clone().into_iter().collect();
                    v.sort_by(|a, b| a.0.cmp(&b.0));
                    (s.population, s.adults, s.prosperity, s.storage.granary, s.storage.warehouse, s.storage.vault, v)
                })
                .collect();
            fp.sort_by(|a, b| a.0.total_cmp(&b.0));
            (fp, treasury.gold.clone(), trade_state.caravans.len(), trade_state.routes.len())
        };
        assert_eq!(run(), run(), "the same seed must produce an identical M4 world");
    }

    #[test]
    fn pricing_discounts_surplus_and_marks_up_scarcity() {
        let (_, mut towns) = world_and_towns();
        // A seller overflowing with Logs and a buyer starving for them.
        towns[0].resources.insert("Logs".to_string(), 5000.0);
        towns[1].resources.clear();
        let dear = unit_price(&towns[0], &towns[1], "Logs", Season::Spring);
        // Same buyer, but the seller barely has any surplus → price must rise.
        towns[0].resources.insert("Logs".to_string(), 30.0);
        let scarce = unit_price(&towns[0], &towns[1], "Logs", Season::Spring);
        assert!(scarce > dear, "a needy seller must not discount: {scarce} vs {dear}");
        // A full buyer (stock ≥ cap) has zero need → minimum markup.
        towns[1].resources.insert("Logs".to_string(), 5000.0);
        let full = unit_price(&towns[0], &towns[1], "Logs", Season::Spring);
        assert!(full <= dear + 1e-9, "a full buyer must not pay markup: {full}");
    }

    #[test]
    fn safety_reserve_keeps_need_and_floor() {
        let (_, towns) = world_and_towns();
        let s = &towns[0];
        let food_need = needs(s, Season::Summer).get("Food").copied().unwrap_or(0.0);
        let reserve = safety_reserve(s, "Wheat", Season::Summer);
        assert!((reserve - food_need * SAFETY_RESERVE_TURNS).abs() < 1e-9, "food reserve = needs×8");
        // Luxury is a *needed* good too (there is a Luxury need), so Wine
        // keeps a needs×8 reserve like food — the Python `_safety_reserve`
        // rule "food/needed goods → needs_total × reserve_turns".
        let luxury_need = needs(s, Season::Summer).get("Luxury").copied().unwrap_or(0.0);
        let reserve = safety_reserve(s, "Wine", Season::Summer);
        assert!((reserve - luxury_need * SAFETY_RESERVE_TURNS).abs() < 1e-9, "luxury reserve = needs×8");
        // A durable with no need (Stone — no need, no pool membership) keeps
        // the 10% of storage cap floor.
        let cap = economy::settlement_storage_capacity(&s.storage);
        let reserve = safety_reserve(s, "Stone", Season::Summer);
        assert!((reserve - cap * NON_FOOD_RESERVE_FRACTION).abs() < 1e-9, "durables keep 10% of cap");
    }

    #[test]
    fn caravan_pays_in_two_phases_with_tax() {
        let (map, mut towns) = world_and_towns();
        let mut state = TradeState::new(&map, &towns[0], &towns[1]);
        // Complete the route instantly for the test.
        let path = land_path(&map, towns[0].pos, towns[1].pos).unwrap();
        state.routes.push(Route { cells: path.clone() });
        // Seller has surplus Logs; buyer has gold above the reserve.
        towns[0].resources.insert("Logs".to_string(), 200.0);
        towns[0].resources.insert("Gold".to_string(), 1000.0);
        towns[1].resources.clear();
        towns[1].resources.insert("Gold".to_string(), 5000.0);
        let mut treasury = Treasury::new(2);
        run_trade_ai(&mut state, &mut towns, Season::Spring);
        assert_eq!(state.caravans.len(), 1);
        let (qty, price) = (state.caravans[0].quantity, state.caravans[0].unit_price);
        assert!(qty >= MIN_TRADE_QUANTITY);
        // Travel the outbound leg.
        let days = state.caravans[0].turns_total;
        advance_caravans(&mut state, &mut towns, &mut treasury, days + 0.1);
        assert_eq!(state.caravans.len(), 1, "return leg still travelling");
        let c = &state.caravans[0];
        assert_eq!(c.leg, Leg::Return);
        assert!((towns[1].resources["Logs"] - qty).abs() < 1e-9, "buyer got the goods");
        let paid: f64 = c.payment.iter().map(|(_, q)| q).sum();
        assert!((paid - c.total_price.min(5000.0 - GOLD_TRADE_RESERVE)).abs() < 1e-9);
        // Return home → seller credited, tax skimmed to the treasury.
        let gold_before = towns[0].resources["Gold"];
        let treasury_before = treasury.gold[0];
        advance_caravans(&mut state, &mut towns, &mut treasury, days + 0.1);
        assert!(state.caravans.is_empty(), "caravan retires after payment");
        let tax = paid * crate::build::TRADE_TAX_RATE;
        assert!((towns[0].resources["Gold"] - (gold_before + paid - tax)).abs() < 1e-9);
        assert!((treasury.gold[0] - (treasury_before + tax)).abs() < 1e-9);
        // The goods left the seller at dispatch.
        assert!((towns[0].resources["Logs"] - (200.0 - qty)).abs() < 1e-9);
    }

    #[test]
    fn route_project_finishes_then_caravans_flow() {
        let (map, mut towns) = world_and_towns();
        let mut state = TradeState::new(&map, &towns[0], &towns[1]);
        let clock = SimClock { seconds: 0.0 };
        let mut treasury = Treasury::new(2);
        towns[0].resources.insert("Logs".to_string(), 5000.0);
        towns[0].resources.insert("Gold".to_string(), 5000.0);
        towns[1].resources.insert("Gold".to_string(), 5000.0);
        // Before the route exists, no caravans.
        sim_tick(&mut state, &mut towns, &mut treasury, &clock, 1.0);
        assert_eq!(state.route_projects.len(), 1, "route proposed on day one");
        assert!(state.caravans.is_empty(), "no route yet → no caravans");
        // Build it (both ends, 6 cells/day) then caravans must dispatch.
        let total = state.route_projects[0].total_cells();
        let days = (total / TRADE_ROUTE_CELLS_PER_TURN).ceil() + 1.0;
        let mut seconds = 0.0;
        for _ in 0..(days as usize) {
            seconds += 1.0;
            sim_tick(&mut state, &mut towns, &mut treasury, &SimClock { seconds }, 1.0);
        }
        assert!(state.routes.len() == 1, "route completed");
        assert!(state.route_projects.is_empty());
        assert!(state.caravans.len() >= 1, "caravans flow once the road exists");
        // And the AI caps at MAX_ACTIVE_TRADES_PER_FACTION outbound per faction.
        assert!(state.caravans.iter().filter(|c| c.seller_idx == 0).count() <= MAX_ACTIVE_TRADES_PER_FACTION);
    }
}
