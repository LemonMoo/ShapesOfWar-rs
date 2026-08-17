//! `build` — continuous construction and the kingdom treasury (port
//! milestone M4).
//!
//! M4 is the `build` half of "trade + build": the town's storage buildings
//! (granary / warehouse / vault) and the coin that pays for them. It ports
//! three named pieces of the Python game, rebuilt as continuous systems:
//!
//! - **treasury & currency** — the TAXATION_PLAN model: a central kingdom
//!   treasury per faction (`STARTING_TREASURY_PER_FACTION` seed), an
//!   **income tax** that drains each settlement's held Gold into it at its
//!   rolled `tax_income` rate (`SETTLEMENT_TAX_INCOME["town"]`), and a
//!   **transaction tax** (`TRADE_TAX_RATE`, skimmed in `trade` on payment
//!   credit). Taxes redistribute, never mint: they only move coin that
//!   already exists.
//! - **construction** — `STORAGE_BUILD_COSTS` / `STORAGE_BUILD_TURNS` /
//!   `storage_max_tier`: building a tier of granary/warehouse/vault costs its
//!   table line **up front** (Gold from the treasury, goods drawn from the
//!   faction's stockpiles largest-first), then a `StorageProject` accrues
//!   `progress += dt` at one day per day — no per-day resource draw — and on
//!   completion `economy::pool_capacity` grows by the tier's bonus
//!   (`STORAGE_TIER_BONUS`).
//! - **the storage AI** — `run_storage_ai` (pressure `fill >= 0.8`), one
//!   project per settlement, worst-pressure building first, exactly the
//!   Python decision order.
//!
//! The M3 deferral this resolves: `pool_capacity` was flat because "no
//! storage buildings exist in M3 (that is the M4 build milestone)". The M2
//! deferral it resolves: `tax_income` was 0 "until the currency milestone
//! (M3/M4)" — it is now rolled at spawn and collected daily.

use crate::economy::{self, StorageClass, StorageTiers};
use crate::settlement::Settlement;

// --- treasury (currency milestone — TAXATION_PLAN) --------------------------

/// One-time starting reserve for the crown so turn-1 construction isn't
/// frozen while the first taxes land (`STARTING_TREASURY_PER_FACTION`).
pub const STARTING_TREASURY_PER_FACTION: f64 = 2000.0;

/// The fraction of a Gold payment the crown skims into the seller's
/// treasury when a sale is credited (`TRADE_TAX_RATE` — levied in `trade`).
pub const TRADE_TAX_RATE: f64 = 0.10;

/// A faction's central pot, separate from the coin in its settlements:
/// taxes fill it, and it alone pays the Gold line of construction.
#[derive(Resource, Clone, Debug)]
pub struct Treasury {
    /// Gold per faction, indexed by faction id.
    pub gold: Vec<f64>,
}

impl Treasury {
    pub fn new(n_factions: usize) -> Self {
        Treasury {
            gold: vec![STARTING_TREASURY_PER_FACTION; n_factions],
        }
    }

    /// Try to spend `amount` from `faction`'s treasury; false if it cannot
    /// afford it (and nothing is drawn).
    pub fn pay(&mut self, faction: usize, amount: f64) -> bool {
        if amount <= 0.0 {
            return true;
        }
        if self.gold[faction] < amount {
            return false;
        }
        self.gold[faction] -= amount;
        true
    }

    pub fn credit(&mut self, faction: usize, amount: f64) {
        self.gold[faction] += amount;
    }
}

/// Income tax (`collect_income_tax`, TAXATION_PLAN): each settlement pays up
/// to its rolled `tax_income` rate per day, drawn from the Gold it actually
/// holds — a settlement holding no gold pays nothing, and the faction's
/// total coin (nodes + treasury) never changes. Continuous: rate × dt.
pub fn collect_income_tax<'a>(
    settlements: impl IntoIterator<Item = &'a mut Settlement>,
    treasury: &mut Treasury,
    dt: f64,
) {
    if dt <= 0.0 {
        return;
    }
    for s in settlements.into_iter() {
        if s.tax_income <= 0.0 {
            continue;
        }
        let held = s.resources.get("Gold").copied().unwrap_or(0.0);
        let take = held.min(s.tax_income * dt);
        if take <= 0.0 {
            continue;
        }
        *s.resources.get_mut("Gold").unwrap() -= take;
        treasury.credit(s.faction_idx, take);
    }
}

// --- storage buildings -------------------------------------------------------

/// The three pool buildings a town may build (the `STORAGE_BUILD_COSTS`
/// settlement rows; herd/outstation/preserving buildings are village or
/// under-only and arrive with those milestones).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StorageBuilding {
    Granary,
    Warehouse,
    Vault,
}

impl StorageBuilding {
    pub const ALL: [StorageBuilding; 3] = [
        StorageBuilding::Granary,
        StorageBuilding::Warehouse,
        StorageBuilding::Vault,
    ];

    pub fn name(self) -> &'static str {
        match self {
            StorageBuilding::Granary => "granary",
            StorageBuilding::Warehouse => "warehouse",
            StorageBuilding::Vault => "vault",
        }
    }

    /// Which storage pool the building expands.
    pub fn pool(self) -> StorageClass {
        match self {
            StorageBuilding::Granary => StorageClass::Household,
            StorageBuilding::Warehouse => StorageClass::Durable,
            StorageBuilding::Vault => StorageClass::Other,
        }
    }

    /// Highest tier this node kind can reach — `storage_max_tier`, a
    /// settlement with the pool buildings = `len(STORAGE_TIER_BONUS[i]) - 1`.
    pub fn max_tier(self) -> u8 {
        match self {
            StorageBuilding::Granary => (economy::STORAGE_TIER_BONUS[0].len() - 1) as u8,
            StorageBuilding::Warehouse => (economy::STORAGE_TIER_BONUS[1].len() - 1) as u8,
            StorageBuilding::Vault => (economy::STORAGE_TIER_BONUS[2].len() - 1) as u8,
        }
    }

    /// Build-time days for a tier (`STORAGE_BUILD_TURNS`, "at full speed").
    pub fn turns(self, tier: u8) -> f64 {
        let table: &[f64] = match self {
            StorageBuilding::Granary => &[0.0, 15.0, 22.0, 30.0],
            StorageBuilding::Warehouse => &[0.0, 15.0, 22.0, 30.0],
            StorageBuilding::Vault => &[0.0, 18.0, 28.0],
        };
        table.get(tier as usize).copied().unwrap_or(f64::INFINITY)
    }

    /// The cost line for a tier (`STORAGE_BUILD_COSTS` settlement rows) —
    /// paid up front, before the project starts.
    pub fn cost(self, tier: u8) -> &'static [(&'static str, f64)] {
        match (self, tier) {
            (StorageBuilding::Granary, 1) => &[("Logs", 300.0), ("Stone", 100.0), ("Gold", 150.0)],
            (StorageBuilding::Granary, 2) => {
                &[("Planks", 260.0), ("Bricks", 180.0), ("Stone", 220.0), ("Gold", 420.0)]
            }
            (StorageBuilding::Granary, 3) => {
                &[("Planks", 620.0), ("Bricks", 520.0), ("Tools", 180.0), ("Gold", 1100.0)]
            }
            (StorageBuilding::Warehouse, 1) => &[("Logs", 250.0), ("Stone", 200.0), ("Gold", 150.0)],
            (StorageBuilding::Warehouse, 2) => {
                &[("Planks", 300.0), ("Bricks", 240.0), ("Stone", 260.0), ("Gold", 450.0)]
            }
            (StorageBuilding::Warehouse, 3) => {
                &[("Planks", 700.0), ("Bricks", 600.0), ("Tools", 220.0), ("Gold", 1200.0)]
            }
            (StorageBuilding::Vault, 1) => &[("Stone", 320.0), ("Iron", 120.0), ("Gold", 300.0)],
            (StorageBuilding::Vault, 2) => {
                &[("Stone", 700.0), ("Iron", 300.0), ("Tools", 160.0), ("Gold", 900.0)]
            }
            _ => &[],
        }
    }

    /// The current tier of this building on a settlement.
    pub fn tier_of(self, s: &Settlement) -> u8 {
        match self {
            StorageBuilding::Granary => s.storage.granary,
            StorageBuilding::Warehouse => s.storage.warehouse,
            StorageBuilding::Vault => s.storage.vault,
        }
    }

    fn set_tier(self, tiers: &mut StorageTiers, tier: u8) {
        match self {
            StorageBuilding::Granary => tiers.granary = tier,
            StorageBuilding::Warehouse => tiers.warehouse = tier,
            StorageBuilding::Vault => tiers.vault = tier,
        }
    }
}

/// One building under construction (`StorageProject`): which settlement,
/// which building, which tier, and how many days of a `turns`-day build
/// have elapsed. Payment is up front; only time accrues (`progress += dt`).
#[derive(Clone, Debug)]
pub struct StorageProject {
    pub settlement_id: u32,
    pub building: StorageBuilding,
    pub to_tier: u8,
    pub progress: f64,
}

/// The world's build state (Python `world.storage_projects`).
#[derive(Resource, Default)]
pub struct BuildState {
    pub projects: Vec<StorageProject>,
}

// --- paying for construction (can_afford / _pay_cost, TAXATION_PLAN) --------

/// Goods a faction holds across ALL its settlements (Python
/// `_faction_settlement_stock`): construction "hauls in from wherever it's
/// stockpiled".
fn faction_stock<'a>(
    settlements: impl IntoIterator<Item = &'a Settlement>,
    faction: usize,
    resource: &str,
) -> f64 {
    settlements
        .into_iter()
        .filter(|s| s.faction_idx == faction)
        .map(|s| s.resources.get(resource).copied().unwrap_or(0.0))
        .sum()
}

/// `can_afford`: every cost line must be covered — Gold from the kingdom
/// treasury, everything else from the faction's aggregated settlement stock.
pub fn can_afford<'a>(
    cost: &[(&str, f64)],
    settlements: impl IntoIterator<Item = &'a Settlement>,
    treasury: &Treasury,
    faction: usize,
) -> bool {
    let settlements: Vec<&Settlement> = settlements.into_iter().collect();
    cost.iter().all(|(res, amt)| {
        if *res == "Gold" {
            treasury.gold.get(faction).copied().unwrap_or(0.0) >= *amt
        } else {
            faction_stock(settlements.iter().copied(), faction, res) >= *amt
        }
    })
}

/// `_pay_cost`: draw the Gold line from the treasury; draw every other line
/// from the faction's stockpiles largest-first across its settlements
/// (exactly the Python "hauled in from wherever it's stockpiled" model).
pub fn pay_cost<'a>(
    cost: &[(&str, f64)],
    settlements: impl IntoIterator<Item = &'a mut Settlement>,
    treasury: &mut Treasury,
    faction: usize,
) {
    let mut settlements: Vec<&mut Settlement> = settlements.into_iter().collect();
    for (res, amt) in cost {
        if *amt <= 0.0 {
            continue;
        }
        if *res == "Gold" {
            let _ = treasury.pay(faction, *amt);
            continue;
        }
        let mut remaining = *amt;
        // Largest stockpile first, across the faction's settlements — the
        // same ordering as `consume_from_pool`, so determinism is by size.
        let mut owners: Vec<(usize, f64)> = settlements
            .iter()
            .enumerate()
            .filter(|(_, s)| s.faction_idx == faction)
            .map(|(i, s)| (i, s.resources.get(*res).copied().unwrap_or(0.0)))
            .collect();
        owners.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (i, have) in owners {
            if remaining <= 0.0 {
                break;
            }
            let take = have.min(remaining);
            if take > 0.0 {
                if let Some(stock) = settlements[i].resources.get_mut(*res) {
                    *stock -= take;
                }
                remaining -= take;
            }
        }
        debug_assert!(remaining <= 1e-6, "pay_cost overdrew {}: {remaining:.3} left", res);
    }
}

// --- construction (continuous) -----------------------------------------------

/// Advance every project by `dt` days; a project whose time is up sets its
/// building tier (`set_storage_tier` in the Python) and is removed. No
/// per-day resource draw — the cost was paid up front.
pub fn advance_projects<'a>(
    state: &mut BuildState,
    settlements: impl IntoIterator<Item = &'a mut Settlement>,
    dt: f64,
) {
    if dt <= 0.0 {
        return;
    }
    let mut settlements: Vec<&mut Settlement> = settlements.into_iter().collect();
    let mut done: Vec<usize> = Vec::new();
    for (i, p) in state.projects.iter_mut().enumerate() {
        p.progress += dt;
        if p.progress >= p.building.turns(p.to_tier) {
            done.push(i);
        }
    }
    // Complete in insertion order (deterministic) — one project per
    // settlement, so a settlement can't have two competing completions.
    for (k, &i) in done.iter().enumerate() {
        let p = state.projects.remove(i - k);
        if let Some(s) = settlements.iter_mut().find(|s| s.id == p.settlement_id) {
            let old = p.building.tier_of(s);
            p.building.set_tier(&mut s.storage, p.to_tier);
            println!(
                "[build] {} completed its {} tier {} ({} → {}); {} pool now {}",
                s.name,
                p.building.name(),
                p.to_tier,
                old,
                p.to_tier,
                p.building.name(),
                economy::pool_capacity_tiers(p.building.pool(), &s.storage)
            );
        }
    }
}

// --- the storage AI (run_storage_ai) ----------------------------------------

/// Pressure threshold that makes the AI build (`fill >= 0.8` per pool).
pub const STORAGE_AI_PRESSURE_THRESHOLD: f64 = 0.8;

/// Per-day build decision (Python `run_storage_ai`): every settlement, in
/// id order, may run ONE project at a time; for each building whose next
/// tier is affordable and whose pool is under pressure (fill ≥ 0.8), it
/// starts the build — worst-pressure pool first. One project per settlement
/// is the Python "one major build at a time" gate, scoped to storage.
pub fn run_storage_ai<'a>(
    state: &mut BuildState,
    settlements: impl IntoIterator<Item = &'a mut Settlement>,
    treasury: &mut Treasury,
) {
    let mut settlements: Vec<&mut Settlement> = settlements.into_iter().collect();
    settlements.sort_by(|a, b| a.id.cmp(&b.id));
    for id in settlements.iter().map(|s| s.id).collect::<Vec<u32>>() {
        if state.projects.iter().any(|p| p.settlement_id == id) {
            continue; // this settlement is already building something
        }
        let faction = settlements.iter().find(|s| s.id == id).unwrap().faction_idx;
        // Worst-pressure building first — deterministic: sort by fill desc.
        let mut candidates: Vec<(StorageBuilding, f64)> = StorageBuilding::ALL
            .iter()
            .copied()
            .map(|b| {
                let s = settlements.iter().find(|s| s.id == id).unwrap();
                let fill = economy::pool_stock(&s.resources, b.pool())
                    / economy::pool_capacity_tiers(b.pool(), &s.storage);
                (b, fill)
            })
            .collect();
        candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (building, fill) in candidates {
            if fill < STORAGE_AI_PRESSURE_THRESHOLD {
                break; // below pressure — no building is worth building
            }
            let s = settlements.iter().find(|s| s.id == id).unwrap();
            let cur = building.tier_of(s);
            if cur >= building.max_tier() {
                continue;
            }
            let to_tier = cur + 1;
            let cost = building.cost(to_tier);
            let ro: Vec<&Settlement> = settlements.iter().map(|s| &**s).collect();
            if !can_afford(cost, ro, treasury, faction) {
                continue;
            }
            pay_cost(cost, settlements.iter_mut().map(|s| &mut **s), treasury, faction);
            state.projects.push(StorageProject {
                settlement_id: id,
                building,
                to_tier,
                progress: 0.0,
            });
            println!(
                "[build] {} started a {} tier {} ({} days)",
                settlements.iter().find(|s| s.id == id).unwrap().name,
                building.name(),
                to_tier,
                building.turns(to_tier)
            );
            break;
        }
    }
}

/// One full world-level build tick: tax first (so the treasury is full when
/// development spends it), then finish old projects, then start new ones.
/// Pure and deterministic — the Bevy system, headless tests and examples
/// all call this.
pub fn sim_tick<'a>(
    state: &mut BuildState,
    settlements: impl IntoIterator<Item = &'a mut Settlement>,
    treasury: &mut Treasury,
    dt: f64,
) {
    let mut settlements: Vec<&mut Settlement> = settlements.into_iter().collect();
    settlements.sort_by(|a, b| a.id.cmp(&b.id));
    collect_income_tax(settlements.iter_mut().map(|s| &mut **s), treasury, dt);
    advance_projects(state, settlements.iter_mut().map(|s| &mut **s), dt);
    run_storage_ai(state, settlements.iter_mut().map(|s| &mut **s), treasury);
}

// --- Bevy plugin -------------------------------------------------------------

use bevy::prelude::*;

/// The build plugin: runs the world-level build tick each fixed step, after
/// trade (`crate::trade::trade_system`) so the treasury is full when
/// development spends it.
pub struct BuildPlugin;

impl Plugin for BuildPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, (build_system).after(crate::trade::trade_system));
    }
}

pub(crate) fn build_system(
    mut state: ResMut<BuildState>,
    mut q: Query<&mut Settlement>,
    mut treasury: ResMut<Treasury>,
    time: Res<Time<Fixed>>,
) {
    let mut v: Vec<bevy::ecs::change_detection::Mut<Settlement>> = q.iter_mut().collect();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    let refs: Vec<&mut Settlement> = v.iter_mut().map(|s| &mut **s).collect();
    sim_tick(&mut state, refs, &mut treasury, time.delta_secs_f64());
}

// --- tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen;

    fn two_towns() -> (Vec<Settlement>, Treasury, BuildState) {
        let map = worldgen::generate(256, 160, 2024);
        let a = Settlement::spawn(&map, 2024);
        let b = Settlement::spawn_at(&map, 2024, 1, 1, Some(a.pos));
        let settlements = vec![a, b];
        let treasury = Treasury::new(2);
        (settlements, treasury, BuildState::default())
    }

    #[test]
    fn cost_and_turn_tables_are_complete_and_consistent() {
        // max_tier matches the STORAGE_TIER_BONUS table length, and every
        // tier ≤ max has a finite build time and a non-trivial cost.
        for b in StorageBuilding::ALL {
            assert_eq!(b.max_tier() as usize + 1, match b {
                StorageBuilding::Granary => economy::STORAGE_TIER_BONUS[0].len(),
                StorageBuilding::Warehouse => economy::STORAGE_TIER_BONUS[1].len(),
                StorageBuilding::Vault => economy::STORAGE_TIER_BONUS[2].len(),
            });
            for tier in 1..=b.max_tier() {
                assert!(b.turns(tier).is_finite() && b.turns(tier) > 0.0, "{} t{tier} turns", b.name());
                assert!(!b.cost(tier).is_empty(), "{} t{tier} cost empty", b.name());
                assert!(b.cost(tier).iter().all(|(_, a)| *a > 0.0));
            }
        }
    }

    #[test]
    fn storage_project_completion_expands_the_pool() {
        let (mut settlements, mut treasury, mut state) = two_towns();
        // Seed faction 0 with the goods for a tier-1 granary + treasury gold.
        for s in settlements.iter_mut().filter(|s| s.faction_idx == 0) {
            s.resources.insert("Logs".to_string(), 400.0);
            s.resources.insert("Stone".to_string(), 200.0);
        }
        let before = economy::pool_capacity_tiers(StorageClass::Household, &settlements[0].storage);
        assert_eq!(before, 840.0, "no buildings yet → town base");
        // Pay the granary tier-1 cost up front, then build it directly (the
        // AI's pressure ordering is covered by the AI test below).
        let cost = StorageBuilding::Granary.cost(1);
        assert!(can_afford(cost, &settlements, &treasury, 0));
        pay_cost(cost, &mut settlements, &mut treasury, 0);
        state.projects.push(StorageProject {
            settlement_id: 0,
            building: StorageBuilding::Granary,
            to_tier: 1,
            progress: 0.0,
        });
        // Cost was paid: Logs/Stone drawn largest-first, Gold from treasury.
        assert_eq!(settlements[0].resources["Logs"], 400.0 - 300.0);
        assert_eq!(settlements[0].resources["Stone"], 200.0 - 100.0);
        assert!((treasury.gold[0] - (STARTING_TREASURY_PER_FACTION - 150.0)).abs() < 1e-9);
        // Build takes 15 days, then the pool capacity grows by 1200.
        advance_projects(&mut state, &mut settlements, 14.9);
        assert_eq!(economy::pool_capacity_tiers(StorageClass::Household, &settlements[0].storage), 840.0);
        advance_projects(&mut state, &mut settlements, 0.2);
        assert!(state.projects.is_empty());
        assert_eq!(
            economy::pool_capacity_tiers(StorageClass::Household, &settlements[0].storage),
            840.0 + 1200.0
        );
    }

    #[test]
    fn ai_builds_only_under_pressure_and_one_at_a_time() {
        let (mut settlements, mut treasury, mut state) = two_towns();
        // Pools below pressure → nothing built.
        run_storage_ai(&mut state, &mut settlements, &mut treasury);
        assert!(state.projects.is_empty(), "empty pools must not trigger builds");
        // Flood the durable pool (Logs bulk 3.0 → 1000 Logs = 3000 space vs
        // a 750 base) so warehouse is the worst pressure.
        for s in settlements.iter_mut() {
            s.resources.insert("Logs".to_string(), 1000.0);
            s.resources.insert("Stone".to_string(), 500.0);
            s.resources.insert("Iron".to_string(), 500.0);
        }
        run_storage_ai(&mut state, &mut settlements, &mut treasury);
        assert_eq!(state.projects.len(), 2, "both towns build their own warehouse");
        assert!(state.projects.iter().all(|p| p.building == StorageBuilding::Warehouse && p.to_tier == 1));
        // The same settlement can't start a second project while one runs.
        run_storage_ai(&mut state, &mut settlements, &mut treasury);
        assert_eq!(state.projects.len(), 2, "one project per settlement");
    }

    #[test]
    fn income_tax_is_redistribution() {
        let (mut settlements, mut treasury, _) = two_towns();
        for s in settlements.iter_mut() {
            s.resources.insert("Gold".to_string(), 500.0);
        }
        let gold_before: f64 = settlements.iter().map(|s| s.resources["Gold"]).sum();
        let treasury_before: f64 = treasury.gold.iter().sum();
        collect_income_tax(&mut settlements, &mut treasury, 10.0);
        let gold_after: f64 = settlements.iter().map(|s| s.resources["Gold"]).sum();
        let treasury_after: f64 = treasury.gold.iter().sum();
        // No minting: total coin is unchanged.
        assert!((gold_before + treasury_before - gold_after - treasury_after).abs() < 1e-9);
        // And a settlement with no gold pays nothing.
        for s in settlements.iter_mut() {
            s.resources.insert("Gold".to_string(), 0.0);
        }
        let t = treasury.gold.iter().sum::<f64>();
        collect_income_tax(&mut settlements, &mut treasury, 10.0);
        assert!((treasury.gold.iter().sum::<f64>() - t).abs() < 1e-9);
    }

    #[test]
    fn treasury_funds_construction() {
        let (mut settlements, mut treasury, _) = two_towns();
        for s in settlements.iter_mut().filter(|s| s.faction_idx == 0) {
            s.resources.insert("Stone".to_string(), 500.0);
            s.resources.insert("Iron".to_string(), 300.0);
        }
        let cost = StorageBuilding::Vault.cost(1);
        assert!(can_afford(cost, &settlements, &treasury, 0), "treasury seed must cover Gold 300");
        // No gold in the treasury → cannot afford.
        treasury.gold[0] = 0.0;
        assert!(!can_afford(cost, &settlements, &treasury, 0));
        // Faction 1's stock can't pay faction 0's build.
        treasury.gold[0] = STARTING_TREASURY_PER_FACTION;
        settlements[1].resources.insert("Stone".to_string(), 9999.0);
        settlements[0].resources.clear();
        assert!(!can_afford(cost, &settlements, &treasury, 0));
    }
}
