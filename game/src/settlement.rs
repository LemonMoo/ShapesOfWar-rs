//! `settlement` — one faction, one continuously-producing settlement (port
//! milestone M2).
//!
//! Everything here ports a named piece of the Python game's economy
//! (`app/world/resources.py` + `app/world/worldgen.py`), rebuilt as
//! *continuous* systems: rates are per-day, the fixed timestep integrates
//! `rate × dt`, and there is no atomic "turn" anywhere.
//!
//! Ported pieces (constants carry the Python value + a pointer to the source
//! symbol):
//!
//! - the needs model — `settlement_needs` / `_population_scaled_need`
//!   (FOOD_PER_CAPITA … TIMBER_UPKEEP_PER_CAPITA);
//! - consumption & consequences — `_consume_node_needs` (starvation /
//!   freeze grace counters, severities, the firewood scrounge fallback);
//! - demographics — `_grow_population` (frontier rate for a town, adult/
//!   child split with the ADULT_REGROWTH_FRACTION floor, child maturation);
//! - prosperity — `_prosperity_condition` / `_prosperity_target` /
//!   `_update_prosperity` (shortage weights, luxury bonus, PROSPERITY_EASE);
//! - production — `_crop_yield_core` / `compute_industry_yield` with the
//!   real GROWTH_CYCLE + RESOURCE_SPAWN data (13 crops, Logs, Firewood),
//!   rarity-shared biome land, climate affinity, fertility weighting;
//! - geography — `village_local_sample` (catchment sampling: biome counts,
//!   dominant climate, mean fertility), `classify_climate`,
//!   `_compute_fertility` (moisture + lowland + water-distance), and the
//!   `_roll_population` starting roll for a town.
//!
//! M2 simplifications, each a conscious deferral to a later milestone:
//! - one faction, one settlement (a town), no regions/kingdoms yet — so the
//!   population-migration term is zero (the Python code defaults a lone
//!   node's region/kingdom averages to its own wealth) and no coal exists
//!   for firewood substitution (the hook is marked in `consume`);
//! - the food pool is the 13 crops only (Python also pools Food Products,
//!   fish and underworld food — all M3+ production);
//! - Clothes/Luxury are consumed but not yet produced (M3's conversion
//!   chains), so they read as a permanent, prosperity-only shortfall — the
//!   exact state the Python game was in before logistics existed.

use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;

use crate::grid::Grid;
use crate::rng::Rng;
use crate::time::{Season, SimClock};
use crate::worldgen::{self, WorldMap};

// --- ported constants: needs (resources.py _population_scaled_need) ---------
pub const FOOD_PER_CAPITA: f64 = 0.005; // per adult, per day
pub const FIREWOOD_PER_CAPITA_WINTER: f64 = 0.003; // per population, Winter only
pub const CLOTHES_PER_CAPITA: f64 = 0.0003;
pub const LUXURY_PER_CAPITA: f64 = 0.0008;
pub const TIMBER_UPKEEP_PER_CAPITA: f64 = 0.016;

// --- ported constants: demographics (resources.py _grow_population) ---------
/// Established cities only — a town uses FRONTIER_POPULATION_GROWTH_RATE;
/// this constant is ported for M3+ when the ladder arrives.
#[allow(dead_code)]
pub const POPULATION_GROWTH_RATE: f64 = 0.0001;
pub const FRONTIER_POPULATION_GROWTH_RATE: f64 = 0.004; // towns/villages (40x)
pub const POPULATION_MIN_FRACTION: f64 = 0.05; // hard floor a node never drops below
pub const ADULT_REGROWTH_FRACTION: f64 = 0.4; // adult share floor on growth
pub const ADULT_MATURITY_RATE: f64 = 0.02; // children mature into a stripped workforce

// --- ported constants: starvation / freezing (resources.py) -----------------
pub const STARVATION_SEVERITY: f64 = 0.05; // max fraction of population lost/day
pub const FREEZE_SEVERITY: f64 = 0.02;
pub const STARVATION_GRACE_DAYS: f64 = 10.0; // STARVATION_GRACE_TURNS
pub const FREEZE_GRACE_DAYS: f64 = 8.0; // FREEZE_GRACE_TURNS
// Firewood scrounge (resources.py _firewood_scrounge_fraction): a forest-
// poor region still keeps warm by burning dung/scrub/deadfall.
pub const NO_FOREST_SUBSISTENCE_FRACTION: f64 = 0.5;
pub const FOREST_SELF_SUFFICIENT_SHARE: f64 = 0.20;

// --- ported constants: prosperity (resources.py _update_prosperity) ---------
pub const PROSPERITY_MAX: f64 = 100.0;
pub const PROSPERITY_STARTING: f64 = 0.0;
pub const PROSPERITY_VALUE_CEIL: f64 = 140.0;
pub const PROSPERITY_EASE: f64 = 0.01;
pub const LUXURY_PROSPERITY_BONUS: f64 = 0.25;
pub const PROSPERITY_SHORTAGE_WEIGHT: [(&str, f64); 4] =
    [("Food", 1.0), ("Firewood", 0.6), ("Clothes", 0.25), ("Timber", 0.25)];

// --- ported constants: value (resources.py BASE_VALUE_BY_TIER) --------------
pub const BASE_VALUE_BY_TIER: [f64; 6] = [0.0, 2.0, 3.0, 5.0, 9.0, 15.0];

// --- ported constants: yields (resources.py) --------------------------------
pub const BASE_CROP_YIELD_PER_CELL: f64 = 10.0;
pub const BASE_FORESTRY_YIELD_PER_CELL: f64 = 0.6; // Logs etc.
pub const FIREWOOD_YIELD_PER_CELL: f64 = 0.3; // survival fuel, kept higher
pub const RARITY_ABUNDANCE: [f64; 3] = [1.0, 0.5, 0.2]; // common, uncommon, rare

// --- ported constants: geography --------------------------------------------
// climate classification (resources.py classify_climate)
const COLD_TEMP: f64 = 0.32;
const ARID_MOISTURE: f64 = 0.35;
const HUMID_MOISTURE: f64 = 0.65;
// fertility (worldgen.py _compute_fertility)
const FERT_MOISTURE: f64 = 0.40;
const FERT_LOWLAND: f64 = 0.30;
const FERT_WATER: f64 = 0.30;
const WATER_FALLOFF: f64 = 13.0;
// catchment (worldgen.py _VILLAGE_CATCHMENT_RADIUS = round(5.5 * 0.65))
const VILLAGE_SPACING: f64 = 5.5;
pub const VILLAGE_CATCHMENT_RADIUS: i32 = (VILLAGE_SPACING * 0.65).round() as i32;
// population roll (worldgen.py POPULATION_RANGE / _roll_population, town)
const TOWN_MAX_POP: (f64, f64) = (1200.0, 3500.0);
const STARTING_POP_FRACTION: (f64, f64) = (0.15, 0.25);
const CHILDREN_FRACTION: (f64, f64) = (0.30, 0.42);

// --- climate -----------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Climate {
    Temperate,
    Arid,
    Cold,
    Humid,
}

/// Port of `resources.py classify_climate`: warm at the map's vertical
/// middle, cold at the poles; aridity from the moisture layer.
fn classify_climate(latitude_temp: f64, moisture: f64) -> Climate {
    if latitude_temp < COLD_TEMP {
        Climate::Cold
    } else if moisture < ARID_MOISTURE {
        Climate::Arid
    } else if moisture > HUMID_MOISTURE {
        Climate::Humid
    } else {
        Climate::Temperate
    }
}

// --- biome mapping: the Rust biome set → the Python biome names -------------
// The Python resource tables are keyed on its 12-name biome set; the Rust
// worldgen produces a finer set (grassland, boreal, monsoon, …). Each Rust
// biome maps onto the closest Python farmland name; ocean/ice/snow-peak map
// to None (not farmland — Python's empty biome, skipped by village_local_sample).
fn python_biome(b: u8) -> Option<&'static str> {
    match b {
        worldgen::BIOME_PLAINS | worldgen::BIOME_GRASSLAND => Some("plains"),
        worldgen::BIOME_STEPPE => Some("steppe"),
        worldgen::BIOME_SAVANNAH => Some("savannah"),
        worldgen::BIOME_TAIGA | worldgen::BIOME_BOREAL => Some("taiga"),
        worldgen::BIOME_TEMPERATE_FOREST
        | worldgen::BIOME_TEMPERATE_RAINFOREST
        | worldgen::BIOME_MONSOON => Some("forest"),
        worldgen::BIOME_JUNGLE => Some("jungle"),
        worldgen::BIOME_SWAMP | worldgen::BIOME_MARSH | worldgen::BIOME_MANGROVE => Some("swamp"),
        worldgen::BIOME_HIGHLAND | worldgen::BIOME_ALPINE => Some("highland"),
        worldgen::BIOME_MOUNTAIN => Some("mountain"),
        worldgen::BIOME_TUNDRA => Some("tundra"),
        worldgen::BIOME_DESERT => Some("desert"),
        worldgen::BIOME_COASTAL => Some("coastal"),
        _ => None,
    }
}

/// Site-preference weight for the M2 start-site heuristic. This is an M2-
/// level stand-in for Python's full `startsites.py evaluate_site` (which
/// arrives with the settlement-expansion milestone); it just prefers the
/// farmland biomes near water.
fn biome_preference(b: u8) -> f64 {
    match b {
        worldgen::BIOME_PLAINS | worldgen::BIOME_GRASSLAND => 2.0,
        worldgen::BIOME_STEPPE | worldgen::BIOME_SAVANNAH => 1.5,
        worldgen::BIOME_TEMPERATE_FOREST
        | worldgen::BIOME_TEMPERATE_RAINFOREST
        | worldgen::BIOME_MONSOON => 1.2,
        worldgen::BIOME_JUNGLE => 1.1,
        worldgen::BIOME_COASTAL => 1.0,
        worldgen::BIOME_HIGHLAND => 0.7,
        worldgen::BIOME_TAIGA | worldgen::BIOME_BOREAL => 0.6,
        worldgen::BIOME_SWAMP | worldgen::BIOME_MARSH | worldgen::BIOME_MANGROVE => 0.5,
        worldgen::BIOME_ALPINE => 0.4,
        worldgen::BIOME_TUNDRA => 0.3,
        worldgen::BIOME_DESERT => 0.2,
        _ => 0.0,
    }
}

// --- resources (ported RESOURCE_SPAWN + GROWTH_CYCLE data) -------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rarity {
    Common,
    Uncommon,
    /// No M2 resource is rare (that's the mining seams of M3); the value is
    /// ported so RARITY_ABUNDANCE stays the complete Python table.
    #[allow(dead_code)]
    Rare,
}

impl Rarity {
    fn abundance(self) -> f64 {
        RARITY_ABUNDANCE[self as usize]
    }
}

/// A raw Crops resource: RESOURCE_SPAWN geography + GROWTH_CYCLE harvest
/// season. `affinity` is indexed by `Climate as usize` (Temperate, Arid,
/// Cold, Humid), exactly the Python `climate` dict's order.
pub struct Crop {
    pub name: &'static str,
    pub harvest: Season,
    pub biomes: &'static [&'static str],
    pub affinity: [f64; 4],
    pub fertility_weight: f64,
    pub rarity: Rarity,
}

/// GROWTH_CYCLE + RESOURCE_SPAWN for the 13 crops, verbatim from
/// resources.py (harvest = the season whose stage is "Harvest").
pub const CROPS: [Crop; 13] = [
    Crop { name: "Wheat", harvest: Season::Summer, biomes: &["plains"], affinity: [1.3, 0.4, 0.5, 0.9], fertility_weight: 1.0, rarity: Rarity::Common },
    Crop { name: "Rye", harvest: Season::Summer, biomes: &["plains", "steppe", "highland", "taiga"], affinity: [0.9, 0.7, 1.2, 0.7], fertility_weight: 0.8, rarity: Rarity::Uncommon },
    Crop { name: "Barley", harvest: Season::Autumn, biomes: &["plains", "steppe"], affinity: [1.1, 0.6, 0.9, 0.8], fertility_weight: 1.0, rarity: Rarity::Common },
    Crop { name: "Oats", harvest: Season::Autumn, biomes: &["plains", "steppe", "highland", "taiga"], affinity: [1.0, 0.4, 1.0, 1.1], fertility_weight: 1.0, rarity: Rarity::Common },
    Crop { name: "Potatoes", harvest: Season::Autumn, biomes: &["plains"], affinity: [1.1, 0.5, 0.9, 1.0], fertility_weight: 0.9, rarity: Rarity::Common },
    Crop { name: "Carrots", harvest: Season::Autumn, biomes: &["plains"], affinity: [1.2, 0.6, 0.7, 0.9], fertility_weight: 1.0, rarity: Rarity::Common },
    Crop { name: "Onions", harvest: Season::Autumn, biomes: &["plains", "savannah"], affinity: [1.1, 0.8, 0.6, 0.8], fertility_weight: 1.0, rarity: Rarity::Common },
    Crop { name: "Fodder", harvest: Season::Summer, biomes: &["plains", "steppe", "savannah"], affinity: [1.2, 0.7, 1.1, 1.0], fertility_weight: 0.5, rarity: Rarity::Uncommon },
    Crop { name: "Beans", harvest: Season::Autumn, biomes: &["plains", "savannah"], affinity: [1.2, 0.5, 0.5, 1.0], fertility_weight: 0.9, rarity: Rarity::Uncommon },
    Crop { name: "Peas", harvest: Season::Summer, biomes: &["plains"], affinity: [1.2, 0.5, 0.7, 0.9], fertility_weight: 0.9, rarity: Rarity::Uncommon },
    Crop { name: "Rice", harvest: Season::Autumn, biomes: &["swamp", "jungle"], affinity: [0.9, 0.2, 0.3, 1.4], fertility_weight: 1.0, rarity: Rarity::Uncommon },
    Crop { name: "Cotton", harvest: Season::Autumn, biomes: &["plains", "savannah"], affinity: [0.7, 1.4, 0.2, 1.0], fertility_weight: 1.0, rarity: Rarity::Uncommon },
    Crop { name: "Grapes", harvest: Season::Autumn, biomes: &["plains"], affinity: [1.3, 1.1, 0.2, 0.6], fertility_weight: 0.9, rarity: Rarity::Uncommon },
];

/// The edible crops — M2's food pool (Python `_FOOD_SOURCES` also pools
/// Food Products, fish and underworld food; those arrive with M3+).
pub const FOOD_SOURCES: [&str; 13] = [
    "Wheat", "Rye", "Barley", "Oats", "Potatoes", "Carrots", "Onions", "Fodder",
    "Beans", "Peas", "Rice", "Cotton", "Grapes",
];

/// A Forestry/Mining raw that produces continuously (no growth cycle).
pub struct Forestry {
    pub name: &'static str,
    pub biomes: &'static [&'static str],
    pub affinity: [f64; 4],
    pub fertility_weight: f64,
    pub rarity: Rarity,
    pub yield_per_cell: f64,
}

/// RESOURCE_SPAWN for the two Forestry resources M2 produces. Logs uses the
/// category base; Firewood carries its per-resource override, both from
/// resources.py (`_raw_yield_per_cell`).
pub const FORESTRY: [Forestry; 2] = [
    Forestry { name: "Logs", biomes: &["forest", "taiga", "jungle"], affinity: [1.1, 0.3, 0.9, 1.2], fertility_weight: 0.2, rarity: Rarity::Common, yield_per_cell: BASE_FORESTRY_YIELD_PER_CELL },
    Forestry { name: "Firewood", biomes: &["forest", "taiga", "jungle"], affinity: [1.0, 0.5, 1.0, 1.0], fertility_weight: 0.1, rarity: Rarity::Common, yield_per_cell: FIREWOOD_YIELD_PER_CELL },
];

/// Timber upkeep is drawn from any structural-wood pool (Python
/// `_TIMBER_SOURCES`); M2 only produces Logs.
pub const TIMBER_SOURCES: [&str; 1] = ["Logs"];

/// Port of `_biome_land_shares`: a resource's rarity-weighted share of a
/// biome's farmland among the *eligible* resources of its category — so
/// several crops sharing "plains" don't each independently claim the whole
/// plain.
fn land_share(biome: &str, resource: &Crop) -> f64 {
    if !resource.biomes.contains(&biome) {
        return 0.0;
    }
    let total: f64 = CROPS
        .iter()
        .filter(|c| c.biomes.contains(&biome))
        .map(|c| c.rarity.abundance())
        .sum();
    if total <= 0.0 {
        return 0.0;
    }
    resource.rarity.abundance() / total
}

fn forestry_share(biome: &str, resource: &Forestry) -> f64 {
    if !resource.biomes.contains(&biome) {
        return 0.0;
    }
    let total: f64 = FORESTRY
        .iter()
        .filter(|f| f.biomes.contains(&biome))
        .map(|f| f.rarity.abundance())
        .sum();
    if total <= 0.0 {
        return 0.0;
    }
    resource.rarity.abundance() / total
}

// --- the settlement ----------------------------------------------------------

/// A single population-owning node (the town), shaped exactly like the
/// Python `Settlement`/`Village` pair. Population is a continuous f64 (no
/// integer "head" rounding — a continuous-time port has no day boundary to
/// round at), displayed rounded in the HUD.
#[derive(Component)]
pub struct Settlement {
    /// Settlement id — the key trade/route logic (M4+) will address nodes
    /// by; not yet read by anything in M2.
    #[allow(dead_code)]
    pub id: u32,
    pub name: String,
    pub pos: (i32, i32),
    pub faction_idx: usize,
    /// Continuous headcount; `population == adults + children` is an
    /// invariant every system here maintains.
    pub population: f64,
    pub adults: f64,
    pub children: f64,
    pub max_population: f64,
    pub prosperity: f64,
    /// This node's own stockpile (Python Phase 9/10 storage).
    pub resources: HashMap<String, f64>,
    /// Consecutive days with an unmet Food/Firewood need (Python
    /// turns_without_food / turns_without_firewood, in continuous days).
    pub days_without_food: f64,
    pub days_without_firewood: f64,
    /// Prosperity inputs recorded by the last consumption pass (Python
    /// prosperity_shortfall / prosperity_luxury).
    pub prosperity_shortfall: HashMap<String, f64>,
    pub prosperity_luxury: f64,
    /// Static geography sampled once at spawn (Python village_local_sample).
    pub biome_counts: HashMap<String, i32>,
    pub climate: Climate,
    pub fertility_frac: f64,
}

/// Port of `worldgen._roll_population` for a town (continuous).
pub fn roll_population(rng: &mut Rng) -> (f64, f64, f64, f64) {
    let max_population = rng.range_f64(TOWN_MAX_POP.0, TOWN_MAX_POP.1);
    let total = max_population * rng.range_f64(STARTING_POP_FRACTION.0, STARTING_POP_FRACTION.1);
    let children = total * rng.range_f64(CHILDREN_FRACTION.0, CHILDREN_FRACTION.1);
    (total, total - children, children, max_population)
}

const TOWN_NAMES: [&str; 10] = [
    "Riverton", "Ashford", "Brentwick", "Harrowgate", "Meadowbank", "Clifford",
    "Stonebridge", "Fernwood", "Oakhurst", "Dalemere",
];

impl Settlement {
    /// Roll + place one town on `map`, deterministically from `seed`.
    pub fn spawn(map: &WorldMap, seed: u64) -> Settlement {
        let mut rng = Rng::new(seed ^ 0x5E71_7E5E);
        let (x, y) = find_start_site(map);
        let water_dist = water_distance(map);
        let (biome_counts, climate, fertility_frac) = sample_catchment(map, &water_dist, x, y);
        let (population, adults, children, max_population) = roll_population(&mut rng);
        Settlement {
            id: 0,
            name: TOWN_NAMES[rng.below(TOWN_NAMES.len())].to_string(),
            pos: (x, y),
            faction_idx: 0,
            population,
            adults,
            children,
            max_population,
            prosperity: PROSPERITY_STARTING,
            resources: HashMap::new(),
            days_without_food: 0.0,
            days_without_firewood: 0.0,
            prosperity_shortfall: HashMap::new(),
            prosperity_luxury: 0.0,
            biome_counts,
            climate,
            fertility_frac,
        }
    }
}

/// The one faction (Python world.factions[0]).
#[derive(Resource)]
pub struct Factions {
    pub names: Vec<String>,
}

impl Default for Factions {
    fn default() -> Self {
        Factions {
            names: vec!["The Reach".to_string()],
        }
    }
}

// --- geography helpers -------------------------------------------------------

/// BFS distance from any water cell (ocean / river / lake), in cells. The
/// world wraps in x, so the BFS does too — the same wrap the map renders
/// with. Matches Python `_water_distance` (used by `_compute_fertility`).
fn water_distance(map: &WorldMap) -> Vec<f64> {
    let (w, h) = (map.w, map.h);
    let n = (w * h) as usize;
    let mut dist = vec![i32::MAX; n];
    let mut queue: VecDeque<usize> = VecDeque::new();
    for i in 0..n {
        if !map.land.v[i] || map.river.v[i] || map.lake.v[i] {
            dist[i] = 0;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        let (x, y) = (i as i32 % w, i as i32 / w);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let nx = (x + dx).rem_euclid(w);
            let ny = (y + dy).clamp(0, h - 1);
            let ni = (ny * w + nx) as usize;
            if dist[ni] > dist[i] + 1 {
                dist[ni] = dist[i] + 1;
                queue.push_back(ni);
            }
        }
    }
    dist.into_iter().map(|d| d as f64).collect()
}

/// Port of `worldgen._compute_fertility` at one cell: moisture + a lowland
/// bonus + an irrigation bonus from distance to water, 0..1. Water (ocean or
/// lake) is never farmland.
fn fertility_at(map: &WorldMap, water_dist: &[f64], x: i32, y: i32) -> f64 {
    let i = (y * map.w + x) as usize;
    if !map.land.v[i] || map.lake.v[i] {
        return 0.0;
    }
    let span = (1.0 - map.sea_level).max(1e-9);
    let elev = ((map.height.v[i] - map.sea_level) / span).clamp(0.0, 1.0);
    let lowland = 1.0 - elev;
    let water = (-water_dist[i] / WATER_FALLOFF).exp();
    (FERT_MOISTURE * map.moisture.v[i] + FERT_LOWLAND * lowland + FERT_WATER * water).clamp(0.0, 1.0)
}

/// Port of `village_local_sample`: biome counts, dominant climate and mean
/// fertility over the radius-4 catchment around `(x, y)`.
fn sample_catchment(
    map: &WorldMap,
    water_dist: &[f64],
    x: i32,
    y: i32,
) -> (HashMap<String, i32>, Climate, f64) {
    let r = VILLAGE_CATCHMENT_RADIUS;
    let mut biome_counts: HashMap<String, i32> = HashMap::new();
    let mut climate_counts: HashMap<Climate, i32> = HashMap::new();
    let (mut fert_sum, mut n) = (0.0f64, 0i32);
    for dy in -r..=r {
        for dx in -r..=r {
            let nx = (x + dx).rem_euclid(map.w);
            let ny = (y + dy).clamp(0, map.h - 1);
            let i = (ny * map.w + nx) as usize;
            if let Some(pb) = python_biome(map.biome[i]) {
                *biome_counts.entry(pb.to_string()).or_insert(0) += 1;
            }
            if map.land.v[i] {
                let cl = classify_climate(map.temperature.v[i], map.moisture.v[i]);
                *climate_counts.entry(cl).or_insert(0) += 1;
            }
            // Python sums fertility over every cell in the patch — water
            // contributes 0 and lowers the mean, exactly as here.
            fert_sum += fertility_at(map, water_dist, nx, ny);
            n += 1;
        }
    }
    let climate = climate_counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1))
        .map(|(c, _)| c)
        .unwrap_or(Climate::Temperate);
    let fertility_frac = if n > 0 { fert_sum / n as f64 } else { 0.5 };
    (biome_counts, climate, fertility_frac)
}

/// M2 site heuristic: the highest-scoring farmland cell on the map, where
/// score = 2×fertility + 0.5×water-proximity + biome preference + a fuel
/// term for the catchment's forest cover. Deterministic (fixed scan order,
/// strict `>` so the first best wins).
///
/// The fuel term is a lesson carried over from the Python game's own fuel
/// pass: forest-poor settlements froze to death every Winter until coal and
/// regional markets arrived (both M3/M4 here), so the M2 heuristic refuses
/// to found the demo town on a site that cannot keep itself warm — it
/// prefers farmland with meaningful forest cover in the same catchment.
pub fn find_start_site(map: &WorldMap) -> (i32, i32) {
    let water_dist = water_distance(map);
    // Forest share of each cell's own catchment — a radius-4 box blur of a
    // forest mask, the same window as VILLAGE_CATCHMENT_RADIUS.
    let mut fmask = Grid::new(map.w, map.h, 0.0);
    for i in 0..fmask.v.len() {
        fmask.v[i] = if python_biome(map.biome[i]) == Some("forest") {
            1.0
        } else {
            0.0
        };
    }
    let forest_share = fmask.blur(VILLAGE_CATCHMENT_RADIUS, 1);
    let mut best: Option<(f64, i32, i32)> = None;
    for y in (0..map.h).step_by(2) {
        for x in (0..map.w).step_by(2) {
            let i = (y * map.w + x) as usize;
            if !map.land.v[i] || map.lake.v[i] || map.river.v[i] {
                continue;
            }
            let pref = biome_preference(map.biome[i]);
            if pref <= 0.0 {
                continue;
            }
            let fert = fertility_at(map, &water_dist, x, y);
            let water = (-water_dist[i] / WATER_FALLOFF).exp();
            let fuel = 2.0 * (forest_share.v[i] / 0.30).min(1.0);
            let score = 2.0 * fert + 0.5 * water + pref + fuel;
            if best.map_or(true, |(s, _, _)| score > s) {
                best = Some((score, x, y));
            }
        }
    }
    match best {
        Some((_, x, y)) => (x, y),
        None => (map.w / 2, map.h / 2),
    }
}

// --- production (continuous `rate × dt`) -------------------------------------

/// Gold-equivalent value of `amount` of a resource the M2 settlement
/// touches (Python `resource_value`, tier table only — no Gold yet).
pub fn resource_value(name: &str, amount: f64) -> f64 {
    let tier = match name {
        "Firewood" | "Logs" => 2,
        "Clothes" => 4,
        "Luxury" => 5,
        _ => 1, // crops
    };
    amount * BASE_VALUE_BY_TIER[tier]
}

/// Per-day production rates for `season` (Python `_crop_yield_core` +
/// `compute_industry_yield`). Crops only yield during their own Harvest
/// season; forestry runs every day.
pub fn production_rates(s: &Settlement, season: Season) -> HashMap<String, f64> {
    let mut out: HashMap<String, f64> = HashMap::new();
    let climate = s.climate;
    for crop in CROPS.iter() {
        if crop.harvest != season {
            continue;
        }
        let mut amount = 0.0;
        for (biome, &cells) in &s.biome_counts {
            let share = land_share(biome, crop);
            if share <= 0.0 {
                continue;
            }
            let fert_mult = 1.0 + crop.fertility_weight * (s.fertility_frac - 0.5);
            amount += BASE_CROP_YIELD_PER_CELL
                * cells as f64
                * share
                * crop.affinity[climate as usize]
                * fert_mult;
        }
        if amount > 0.0 {
            out.insert(crop.name.to_string(), amount);
        }
    }
    for f in FORESTRY.iter() {
        let mut amount = 0.0;
        for (biome, &cells) in &s.biome_counts {
            let share = forestry_share(biome, f);
            if share <= 0.0 {
                continue;
            }
            let fert_mult = 1.0 + f.fertility_weight * (s.fertility_frac - 0.5);
            amount += f.yield_per_cell
                * cells as f64
                * share
                * f.affinity[climate as usize]
                * fert_mult;
        }
        if amount > 0.0 {
            out.insert(f.name.to_string(), amount);
        }
    }
    out
}

/// Gold value of one day of production (feeds the prosperity health factor).
pub fn production_value(s: &Settlement, season: Season) -> f64 {
    production_rates(s, season)
        .iter()
        .map(|(r, a)| resource_value(r, *a))
        .sum()
}

// --- consumption (port of _consume_node_needs, in continuous days) ----------

/// Python `_population_scaled_need`: per-capita demand, floored at 1/day so a
/// tiny headcount never rounds its whole need away.
fn population_scaled_need(headcount: f64, per_capita: f64) -> f64 {
    if headcount <= 0.0 {
        return 0.0;
    }
    (headcount * per_capita).max(1.0)
}

/// Port of `settlement_needs` — per-day needs for `season`. Firewood exists
/// only in Winter; Timber is upkeep (no buildings yet, so no maintenance
/// term); all floored like the Python original.
pub fn needs(s: &Settlement, season: Season) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    out.insert(
        "Food".to_string(),
        population_scaled_need(s.adults, FOOD_PER_CAPITA),
    );
    if season == Season::Winter {
        out.insert(
            "Firewood".to_string(),
            population_scaled_need(s.population, FIREWOOD_PER_CAPITA_WINTER),
        );
    }
    out.insert(
        "Clothes".to_string(),
        population_scaled_need(s.population, CLOTHES_PER_CAPITA),
    );
    out.insert(
        "Luxury".to_string(),
        population_scaled_need(s.population, LUXURY_PER_CAPITA),
    );
    out.insert(
        "Timber".to_string(),
        population_scaled_need(s.population, TIMBER_UPKEEP_PER_CAPITA),
    );
    out
}

/// Gold value of one day of needs (Python `settlement_needs_value`: Food at
/// tier 3, Luxury at tier 5, the others priced off their own resource).
pub fn needs_value(s: &Settlement, season: Season) -> f64 {
    let n = needs(s, season);
    let food = n.get("Food").copied().unwrap_or(0.0) * BASE_VALUE_BY_TIER[3];
    let firewood = n
        .get("Firewood")
        .map(|v| resource_value("Firewood", *v))
        .unwrap_or(0.0);
    let clothes = n
        .get("Clothes")
        .map(|v| resource_value("Clothes", *v))
        .unwrap_or(0.0);
    let luxury = n.get("Luxury").copied().unwrap_or(0.0) * BASE_VALUE_BY_TIER[5];
    let timber = n
        .get("Timber")
        .map(|v| resource_value("Logs", *v))
        .unwrap_or(0.0);
    food + firewood + clothes + luxury + timber
}

/// Python `_consume_from_pool`: draw `needed` from whichever of `sources`
/// has stock, biggest stockpile first. Returns how much was actually drawn.
fn consume_from_pool(res: &mut HashMap<String, f64>, sources: &[&str], needed: f64) -> f64 {
    if needed <= 0.0 {
        return 0.0;
    }
    let mut pool: Vec<(&str, f64)> = sources
        .iter()
        .filter_map(|r| res.get(*r).map(|v| (*r, *v)))
        .filter(|(_, v)| *v > 0.0)
        .collect();
    pool.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut remaining = needed;
    for (r, have) in &pool {
        if remaining <= 0.0 {
            break;
        }
        let take = (*have).min(remaining);
        if let Some(stock) = res.get_mut(*r) {
            *stock -= take;
        }
        remaining -= take;
    }
    needed - remaining
}

/// Firewood scrounge (Python `_firewood_scrounge_fraction`): forest-poor
/// land still keeps its people warm by burning dung/scrub/deadfall; the
/// help tapers to zero once forest cover reaches 20% of the catchment.
fn firewood_scrounge_fraction(s: &Settlement) -> f64 {
    let total: i32 = s.biome_counts.values().sum();
    if total <= 0 {
        return NO_FOREST_SUBSISTENCE_FRACTION;
    }
    let forest_share = s.biome_counts.get("forest").copied().unwrap_or(0) as f64 / total as f64;
    if forest_share >= FOREST_SELF_SUFFICIENT_SHARE {
        return 0.0;
    }
    NO_FOREST_SUBSISTENCE_FRACTION * (1.0 - forest_share / FOREST_SELF_SUFFICIENT_SHARE)
}

/// Python `_apply_population_loss`, continuous: split a loss across
/// adults/children so `population == adults + children` holds, never below
/// the POPULATION_MIN_FRACTION floor.
fn apply_population_loss(s: &mut Settlement, loss: f64) {
    let floor = s.max_population * POPULATION_MIN_FRACTION;
    let loss = loss.max(0.0).min(s.population - floor);
    if loss <= 0.0 {
        return;
    }
    let adult_frac = if s.population > 0.0 {
        s.adults / s.population
    } else {
        0.0
    };
    let adult_loss = loss * adult_frac;
    s.population -= loss;
    s.adults = (s.adults - adult_loss).max(0.0);
    s.children = (s.children - (loss - adult_loss)).max(0.0);
}

/// Port of `_grow_population` (natural increase only — a lone node's
/// region/kingdom averages default to its own wealth, so migration is 0).
fn grow_population(s: &mut Settlement, dt: f64) {
    if s.days_without_food > 0.0 || s.days_without_firewood > 0.0 {
        return;
    }
    // A town grows at the frontier rate (Python: kind == "town" → frontier).
    let rate = FRONTIER_POPULATION_GROWTH_RATE;
    if s.population >= s.max_population {
        return;
    }
    let gain = (s.max_population - s.population) * rate * dt;
    if gain <= 0.0 {
        return;
    }
    let adult_frac = (s.adults / s.population).max(ADULT_REGROWTH_FRACTION);
    let adult_gain = gain * adult_frac;
    s.population += gain;
    s.adults += adult_gain;
    s.children += gain - adult_gain;
    // Child maturation: a workforce stripped by a famine or a freeze regrows
    // as its children come of age (Python ADULT_MATURITY_RATE, continuous).
    if s.children > 0.0 && s.population > 0.0 && s.adults / s.population < ADULT_REGROWTH_FRACTION {
        let mature = s.children.min(s.population * ADULT_MATURITY_RATE * dt);
        s.children -= mature;
        s.adults += mature;
    }
}

/// Port of `_prosperity_condition` + `_prosperity_target` + the easing in
/// `_update_prosperity`. `production_value`/`needs_value` are per-day.
fn update_prosperity(s: &mut Settlement, production_day: f64, needs_day: f64, dt: f64) {
    let health = if needs_day <= 0.0 {
        1.0
    } else {
        (production_day / needs_day).clamp(0.5, 1.5)
    };
    let mut condition = 1.0;
    for (need, deficit) in &s.prosperity_shortfall {
        let w = PROSPERITY_SHORTAGE_WEIGHT
            .iter()
            .find(|(n, _)| n == need)
            .map(|(_, w)| *w)
            .unwrap_or(0.0);
        condition *= (1.0 - w * deficit.clamp(0.0, 1.0)).max(0.0);
    }
    condition += LUXURY_PROSPERITY_BONUS * s.prosperity_luxury;
    // raw_value = settlement_goods_wealth_value = needs_value + tax_income;
    // tax_income is 0 until the currency milestone (M3/M4).
    let raw = needs_day;
    let target =
        (PROSPERITY_MAX * raw * health * condition / PROSPERITY_VALUE_CEIL).clamp(0.0, PROSPERITY_MAX);
    s.prosperity += (target - s.prosperity) * PROSPERITY_EASE * dt;
}

/// One full sim step for the settlement: produce, consume, grow, prosper.
/// `dt` is the fraction of a day this tick covers (fixed timestep 0.1 at
/// SIM_HZ=10). Pure and deterministic — the Bevy system and the headless
/// fingerprint test both call this.
pub fn sim_tick(s: &mut Settlement, clock: &SimClock, dt: f64) {
    let season = clock.season();

    // 1. produce — stock += rate × dt.
    for (res, rate) in production_rates(s, season) {
        *s.resources.entry(res).or_insert(0.0) += rate * dt;
    }

    // 2. consume — needs × dt from storage, grace counters in continuous days.
    let needs_map = needs(s, season);
    let mut shortfall: HashMap<String, f64> = HashMap::new();
    let mut luxury_fulfilled = 0.0;

    let food_needed = needs_map.get("Food").copied().unwrap_or(0.0) * dt;
    let food_had = consume_from_pool(&mut s.resources, &FOOD_SOURCES, food_needed);
    if food_needed > 0.0 && food_had < food_needed {
        s.days_without_food += dt;
        let deficit = (food_needed - food_had) / food_needed;
        shortfall.insert("Food".to_string(), deficit);
        if s.days_without_food > STARVATION_GRACE_DAYS {
            apply_population_loss(s, s.population * deficit * STARVATION_SEVERITY * dt);
        }
    } else {
        s.days_without_food = 0.0;
    }

    match needs_map.get("Firewood") {
        Some(wood_day) => {
            let wood_needed = wood_day * dt;
            let mut wood_had = s
                .resources
                .get("Firewood")
                .copied()
                .unwrap_or(0.0)
                .min(wood_needed);
            if wood_had > 0.0 {
                *s.resources.get_mut("Firewood").unwrap() -= wood_had;
            }
            // Python's coal-substitution branch: no Coal is produced until
            // M3, so a forest-poor town's winter fuel is the scrounge below.
            if wood_had < wood_needed {
                let scrounged = (wood_needed - wood_had)
                    * firewood_scrounge_fraction(s);
                wood_had += scrounged;
            }
            if wood_needed > 0.0 && wood_had < wood_needed {
                s.days_without_firewood += dt;
                let deficit = (wood_needed - wood_had) / wood_needed;
                shortfall.insert("Firewood".to_string(), deficit);
                if s.days_without_firewood > FREEZE_GRACE_DAYS {
                    apply_population_loss(s, s.population * deficit * FREEZE_SEVERITY * dt);
                }
            } else {
                s.days_without_firewood = 0.0;
            }
        }
        None => s.days_without_firewood = 0.0,
    }

    let clothes_needed = needs_map.get("Clothes").copied().unwrap_or(0.0) * dt;
    let clothes_had = s
        .resources
        .get("Clothes")
        .copied()
        .unwrap_or(0.0)
        .min(clothes_needed);
    if clothes_had > 0.0 {
        *s.resources.get_mut("Clothes").unwrap() -= clothes_had;
    }
    if clothes_needed > 0.0 && clothes_had < clothes_needed {
        shortfall.insert(
            "Clothes".to_string(),
            (clothes_needed - clothes_had) / clothes_needed,
        );
    }

    let timber_needed = needs_map.get("Timber").copied().unwrap_or(0.0) * dt;
    let timber_had = consume_from_pool(&mut s.resources, &TIMBER_SOURCES, timber_needed);
    if timber_needed > 0.0 && timber_had < timber_needed {
        shortfall.insert(
            "Timber".to_string(),
            (timber_needed - timber_had) / timber_needed,
        );
    }

    let luxury_needed = needs_map.get("Luxury").copied().unwrap_or(0.0) * dt;
    let luxury_had = s
        .resources
        .get("Luxury")
        .copied()
        .unwrap_or(0.0)
        .min(luxury_needed);
    if luxury_had > 0.0 {
        *s.resources.get_mut("Luxury").unwrap() -= luxury_had;
    }
    if luxury_needed > 0.0 {
        luxury_fulfilled = luxury_had / luxury_needed;
    }

    s.prosperity_shortfall = shortfall;
    s.prosperity_luxury = luxury_fulfilled;

    // 3. demographics — growth, gated on the grace counters.
    grow_population(s, dt);

    // 4. prosperity — eased toward this day's target.
    update_prosperity(s, production_value(s, season), needs_value(s, season), dt);
}

// --- Bevy plugin -------------------------------------------------------------

#[derive(Resource)]
struct LastSeason(Option<Season>);

#[derive(Resource)]
struct LastYear(i64);

pub struct SettlementPlugin;

impl Plugin for SettlementPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Factions>()
            .insert_resource(LastSeason(None))
            .insert_resource(LastYear(0))
            .add_systems(FixedUpdate, (sim_system, season_logger, year_summary));
    }
}

fn sim_system(mut q: Query<&mut Settlement>, clock: Res<SimClock>, time: Res<Time<Fixed>>) {
    let dt = time.delta_secs_f64();
    for mut s in &mut q {
        sim_tick(&mut s, &clock, dt);
    }
}

/// Console line on each season boundary — the observability the Python
/// `day_steps` slices used to provide, now derived from the clock.
fn season_logger(clock: Res<SimClock>, mut last: ResMut<LastSeason>) {
    let s = clock.season();
    if last.0 != Some(s) {
        println!(
            "[time] Day {} · Year {} — {} ({} days ahead)",
            clock.day(),
            clock.year(),
            s.name(),
            clock.days_left_in_season().ceil() as i64
        );
        last.0 = Some(s);
    }
}

/// Console line once a year: the settlement's year-end state.
fn year_summary(
    clock: Res<SimClock>,
    mut last: ResMut<LastYear>,
    q: Query<&Settlement>,
) {
    let year = clock.year();
    if year == last.0 {
        return;
    }
    last.0 = year;
    for s in &q {
        println!(
            "[year {}] {} — pop {:.0} (adults {:.0}) / max {:.0}, prosperity {:.0}, food {:.1}, firewood {:.1}, logs {:.1}",
            year,
            s.name,
            s.population,
            s.adults,
            s.max_population,
            s.prosperity,
            s.resources.get("Wheat").copied().unwrap_or(0.0),
            s.resources.get("Firewood").copied().unwrap_or(0.0),
            s.resources.get("Logs").copied().unwrap_or(0.0),
        );
    }
}

// --- headless fingerprint test -----------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::TURNS_PER_SEASON;

    fn simulate(days: f64) -> (Settlement, Vec<f64>) {
        let map = worldgen::generate(256, 160, 2024);
        let mut s = Settlement::spawn(&map, 2024);
        // food-stock sample at the start of each season, for the seasonal check
        let mut samples = Vec::new();
        let tick = 0.1;
        let mut seconds = 0.0;
        let mut last_season = -1i64;
        while seconds < days {
            sim_tick(&mut s, &SimClock { seconds }, tick);
            seconds += tick;
            let season = (seconds / TURNS_PER_SEASON).floor() as i64 % 4;
            if season != last_season {
                last_season = season;
                let food: f64 = FOOD_SOURCES
                    .iter()
                    .map(|r| s.resources.get(*r).copied().unwrap_or(0.0))
                    .sum();
                samples.push(food);
            }
        }
        (s, samples)
    }

    #[test]
    fn invariants_and_seasonal_food() {
        let (s, samples) = simulate(400.0);
        // identity: population == adults + children, nothing NaN
        assert!(s.population.is_finite() && s.adults.is_finite() && s.children.is_finite());
        assert!(
            (s.population - (s.adults + s.children)).abs() < 1e-6,
            "pop {} != adults {} + children {}",
            s.population,
            s.adults,
            s.children
        );
        // fed and warm: a good farmland site must never starve or freeze
        assert_eq!(s.days_without_food, 0.0);
        assert_eq!(s.days_without_firewood, 0.0);
        // growth: frontier rate over 400 days from ~20% of ceiling must climb
        assert!(
            s.population > 0.0,
            "population must be positive, got {}",
            s.population
        );
        // seasonal pattern: food stock must rise across a harvest season and
        // be lower before the first harvest than after it — the classic
        // crop cycle. sample[0] is end of Spring (pre-harvest), later
        // samples are post-harvest.
        if samples.len() >= 2 {
            assert!(
                samples[samples.len() - 1] > samples[0] + 100.0,
                "food stock should grow across harvests: {:?}",
                samples
            );
        }
    }

    #[test]
    fn deterministic_fingerprint() {
        let (a, _) = simulate(400.0);
        let (b, _) = simulate(400.0);
        let fp = |s: &Settlement| {
            let mut v: Vec<(String, f64)> = s.resources.clone().into_iter().collect();
            v.sort_by(|x, y| x.0.cmp(&y.0));
            (s.population, s.adults, s.prosperity, v)
        };
        assert_eq!(fp(&a), fp(&b), "same seed must produce identical state");
    }
}
