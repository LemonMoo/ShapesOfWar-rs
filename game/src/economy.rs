//! `economy` — resource registry, conversion, storage, spoilage (port
//! milestone M3).
//!
//! M2 gave the settlement a continuous producer/consumer loop but no real
//! goods model: the food pool was "the 13 crops", Clothes/Luxury were
//! consumed but never produced, and stockpiles grew without limit or decay.
//! M3 ports the Python game's actual economy plumbing (`resources.py`) as
//! continuous systems:
//!
//! - **registry** — the `RESOURCES` table: every resource's category, tier,
//!   `spoil_rate`, `bulk` (storage space per unit), storage `pool`, `edible`
//!   and `luxury` flags, verbatim from `resources.py` (the tables at
//!   `_SPOIL_RATE`, `_CATEGORY_BULK`/`_BULK_OVERRIDES`, `storage_class`);
//! - **conversion** — `RECIPES` + `advance_settlement_production_chains`:
//!   processed goods are converted 1:1 from their recipe inputs at a
//!   per-day rate cap (`CONVERSION_RATE_CAP`, and a far smaller
//!   `LUXURY_CONVERSION_RATE_CAP` that only opens after a full year);
//! - **storage** — per-`pool` capacities (town `STORAGE_POOL_BASE`),
//!   bulk-weighted occupancy, and the `storage_throttle` that tapers a
//!   node's *primary* production as its pool fills (production responds to
//!   storage, so nothing is silently destroyed any more);
//! - **spoilage** — each resource decays at its registry `spoil_rate`, and
//!   a pool packed past capacity decays the overage on top of that
//!   (`OVERFLOW_SPOILAGE_MULTIPLIER` etc.).
//!
//! Pure `HashMap`-over-stock functions, like M2's `sim_tick`: the Bevy
//! system, the headless tests and the examples all call the same code.
//! Livestock and Mining exist in the registry (so the recipes that read
//! them stay faithful) but nothing produces them yet — those recipes can
//! never fire, exactly as in the Python game where "available" for a
//! livestock source is always 0.

use std::collections::HashMap;

// --- registry ----------------------------------------------------------------

/// A resource's category (`RESOURCES` in resources.py).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Crops,
    Livestock,
    Forestry,
    Mining,
    Fishing,
    Subterranean,
    FoodProducts,
    Manufactured,
    Luxury,
}

impl Category {
    fn name(self) -> &'static str {
        match self {
            Category::Crops => "Crops",
            Category::Livestock => "Livestock",
            Category::Forestry => "Forestry",
            Category::Mining => "Mining",
            Category::Fishing => "Fishing",
            Category::Subterranean => "Subterranean",
            Category::FoodProducts => "Food Products",
            Category::Manufactured => "Manufactured Goods",
            Category::Luxury => "Luxury Goods",
        }
    }
}

/// Which storage pool a resource occupies (`storage_class`, Phase 3 of the
/// Python storage rework): a good only ever competes with its own kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StorageClass {
    /// Crops / Food Products / Fishing / Firewood — the Granary pool.
    Household,
    /// Mining / Forestry (except Firewood) / Manufactured — the Warehouse pool.
    Durable,
    /// Luxury Goods, Gold — the Vault pool.
    Other,
    /// Fodder / Manure / Guano — the Barn pool.
    Feed,
}

/// One row of the `RESOURCES` registry with its derived properties filled
/// in, exactly as `resources.py` does at import (`_CATEGORY_PROPERTY_DEFAULTS`
/// + per-resource overrides + `spoil_rate` + `bulk` + `storage_class`).
#[derive(Clone, Copy, Debug)]
pub struct ResourceSpec {
    pub name: &'static str,
    pub category: Category,
    pub tier: u8,
    /// Fraction lost per day in storage (0.0 = never spoils) —
    /// `_SPOIL_RATE`.
    pub spoil_rate: f64,
    /// Storage space one unit occupies relative to a unit of grain —
    /// `_CATEGORY_BULK` + `_BULK_OVERRIDES`.
    pub bulk: f64,
    /// Which storage pool this good competes within — `storage_class`.
    pub pool: StorageClass,
    /// Safe to eat directly (the registry's `edible`, "consumed by mouth"
    /// — broader than "can keep a population alive": Wine and Beer are
    /// edible too).
    pub edible: bool,
    /// A status good, not sustenance — `Luxury Goods` category.
    pub luxury: bool,
}

/// The full Python `RESOURCES` table (60 resources) with every derived
/// property resolved, in `resources.py` order. Mining/Livestock entries are
/// dead weight for M3's town (nothing mines or herds yet) but are the
/// authoritative spec the M4+ milestones read — exactly the way M2 carried
/// the complete tier/rarity tables ahead of their use.
pub const RESOURCES: &[ResourceSpec] = &[
    // --- Crops (tier 1) ---
    ResourceSpec { name: "Wheat", category: Category::Crops, tier: 1, spoil_rate: 0.03, bulk: 1.0, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Barley", category: Category::Crops, tier: 1, spoil_rate: 0.03, bulk: 1.0, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Oats", category: Category::Crops, tier: 1, spoil_rate: 0.03, bulk: 1.0, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Rye", category: Category::Crops, tier: 1, spoil_rate: 0.03, bulk: 1.0, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Potatoes", category: Category::Crops, tier: 1, spoil_rate: 0.06, bulk: 1.0, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Carrots", category: Category::Crops, tier: 1, spoil_rate: 0.07, bulk: 1.0, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Onions", category: Category::Crops, tier: 1, spoil_rate: 0.05, bulk: 1.0, pool: StorageClass::Household, edible: true, luxury: false },
    // Hay/fodder: eaten by animals, not people (`edible: False`), and given
    // its own Barn pool so animal feed never competes with human food.
    ResourceSpec { name: "Fodder", category: Category::Crops, tier: 1, spoil_rate: 0.01, bulk: 2.2, pool: StorageClass::Feed, edible: false, luxury: false },
    ResourceSpec { name: "Beans", category: Category::Crops, tier: 1, spoil_rate: 0.02, bulk: 1.0, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Peas", category: Category::Crops, tier: 1, spoil_rate: 0.02, bulk: 1.0, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Rice", category: Category::Crops, tier: 1, spoil_rate: 0.03, bulk: 1.0, pool: StorageClass::Household, edible: true, luxury: false },
    // Cotton: an industrial fibre (`edible: False`), classed durable so it
    // never competes with food for granary space.
    ResourceSpec { name: "Cotton", category: Category::Crops, tier: 1, spoil_rate: 0.02, bulk: 1.8, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Grapes", category: Category::Crops, tier: 1, spoil_rate: 0.06, bulk: 1.2, pool: StorageClass::Household, edible: true, luxury: false },

    // --- Livestock (tier 1) --- living animals, never stockpiled units ---
    ResourceSpec { name: "Cattle", category: Category::Livestock, tier: 1, spoil_rate: 0.0, bulk: 1.0, pool: StorageClass::Other, edible: false, luxury: false },
    ResourceSpec { name: "Sheep", category: Category::Livestock, tier: 1, spoil_rate: 0.0, bulk: 1.0, pool: StorageClass::Other, edible: false, luxury: false },
    ResourceSpec { name: "Horses", category: Category::Livestock, tier: 1, spoil_rate: 0.0, bulk: 1.0, pool: StorageClass::Other, edible: false, luxury: false },
    ResourceSpec { name: "Goats", category: Category::Livestock, tier: 1, spoil_rate: 0.0, bulk: 1.0, pool: StorageClass::Other, edible: false, luxury: false },
    ResourceSpec { name: "Chickens", category: Category::Livestock, tier: 1, spoil_rate: 0.0, bulk: 1.0, pool: StorageClass::Other, edible: false, luxury: false },
    ResourceSpec { name: "Pigs", category: Category::Livestock, tier: 1, spoil_rate: 0.0, bulk: 1.0, pool: StorageClass::Other, edible: false, luxury: false },
    ResourceSpec { name: "Bees", category: Category::Livestock, tier: 1, spoil_rate: 0.0, bulk: 1.0, pool: StorageClass::Other, edible: false, luxury: false },

    // --- Forestry (tier 2) ---
    ResourceSpec { name: "Logs", category: Category::Forestry, tier: 2, spoil_rate: 0.0, bulk: 3.0, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Hardwood", category: Category::Forestry, tier: 2, spoil_rate: 0.0, bulk: 2.6, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Softwood", category: Category::Forestry, tier: 2, spoil_rate: 0.0, bulk: 2.6, pool: StorageClass::Durable, edible: false, luxury: false },
    // Firewood is a seasonal survival good: classed household (it is fuel,
    // not building material) and exempt from the storage throttle.
    ResourceSpec { name: "Firewood", category: Category::Forestry, tier: 2, spoil_rate: 0.0, bulk: 2.0, pool: StorageClass::Household, edible: false, luxury: false },
    ResourceSpec { name: "Resin", category: Category::Forestry, tier: 2, spoil_rate: 0.02, bulk: 0.8, pool: StorageClass::Durable, edible: false, luxury: false },

    // --- Mining (tier 2) --- nothing here spoils, salt included ---
    ResourceSpec { name: "Iron", category: Category::Mining, tier: 2, spoil_rate: 0.0, bulk: 1.2, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Copper", category: Category::Mining, tier: 2, spoil_rate: 0.0, bulk: 1.2, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Tin", category: Category::Mining, tier: 2, spoil_rate: 0.0, bulk: 1.2, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Coal", category: Category::Mining, tier: 2, spoil_rate: 0.0, bulk: 1.8, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Stone", category: Category::Mining, tier: 2, spoil_rate: 0.0, bulk: 2.5, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Clay", category: Category::Mining, tier: 2, spoil_rate: 0.0, bulk: 2.0, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Sand", category: Category::Mining, tier: 2, spoil_rate: 0.0, bulk: 2.0, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Salt", category: Category::Mining, tier: 2, spoil_rate: 0.0, bulk: 0.8, pool: StorageClass::Durable, edible: true, luxury: false },
    ResourceSpec { name: "Gems", category: Category::Mining, tier: 2, spoil_rate: 0.0, bulk: 0.1, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Gold Ore", category: Category::Mining, tier: 2, spoil_rate: 0.0, bulk: 1.2, pool: StorageClass::Durable, edible: false, luxury: false },

    // --- Fishing (tier 1) ---
    ResourceSpec { name: "Fish", category: Category::Fishing, tier: 1, spoil_rate: 0.35, bulk: 1.0, pool: StorageClass::Household, edible: true, luxury: false },

    // --- Subterranean (tier 1) ---
    ResourceSpec { name: "Mushrooms", category: Category::Subterranean, tier: 1, spoil_rate: 0.18, bulk: 0.9, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Cave Fish", category: Category::Subterranean, tier: 1, spoil_rate: 0.35, bulk: 1.0, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Guano", category: Category::Subterranean, tier: 1, spoil_rate: 0.0, bulk: 1.4, pool: StorageClass::Feed, edible: false, luxury: false },
    ResourceSpec { name: "Manure", category: Category::Subterranean, tier: 1, spoil_rate: 0.01, bulk: 2.2, pool: StorageClass::Feed, edible: false, luxury: false },

    // --- Food Products (tier 3) --- the most perishable tier by far ---
    ResourceSpec { name: "Flour", category: Category::FoodProducts, tier: 3, spoil_rate: 0.05, bulk: 0.9, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Bread", category: Category::FoodProducts, tier: 3, spoil_rate: 0.35, bulk: 1.0, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Meat", category: Category::FoodProducts, tier: 3, spoil_rate: 0.30, bulk: 1.0, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Milk", category: Category::FoodProducts, tier: 3, spoil_rate: 0.40, bulk: 1.2, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Cheese", category: Category::FoodProducts, tier: 3, spoil_rate: 0.05, bulk: 0.8, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Eggs", category: Category::FoodProducts, tier: 3, spoil_rate: 0.15, bulk: 1.2, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Honey", category: Category::FoodProducts, tier: 3, spoil_rate: 0.0, bulk: 0.6, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Smoked Fish", category: Category::FoodProducts, tier: 3, spoil_rate: 0.05, bulk: 0.8, pool: StorageClass::Household, edible: true, luxury: false },
    ResourceSpec { name: "Salted Meat", category: Category::FoodProducts, tier: 3, spoil_rate: 0.03, bulk: 0.9, pool: StorageClass::Household, edible: true, luxury: false },

    // --- Manufactured Goods (tier 4) ---
    ResourceSpec { name: "Planks", category: Category::Manufactured, tier: 4, spoil_rate: 0.0, bulk: 2.0, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Bricks", category: Category::Manufactured, tier: 4, spoil_rate: 0.0, bulk: 2.2, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Glass", category: Category::Manufactured, tier: 4, spoil_rate: 0.0, bulk: 1.0, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Wool", category: Category::Manufactured, tier: 4, spoil_rate: 0.01, bulk: 1.6, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Cloth", category: Category::Manufactured, tier: 4, spoil_rate: 0.0, bulk: 1.0, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Clothes", category: Category::Manufactured, tier: 4, spoil_rate: 0.0, bulk: 1.2, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Leather", category: Category::Manufactured, tier: 4, spoil_rate: 0.0, bulk: 1.2, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Tools", category: Category::Manufactured, tier: 4, spoil_rate: 0.0, bulk: 0.8, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Weapons", category: Category::Manufactured, tier: 4, spoil_rate: 0.0, bulk: 0.8, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Shields", category: Category::Manufactured, tier: 4, spoil_rate: 0.0, bulk: 1.2, pool: StorageClass::Durable, edible: false, luxury: false },
    ResourceSpec { name: "Paper", category: Category::Manufactured, tier: 4, spoil_rate: 0.02, bulk: 0.5, pool: StorageClass::Durable, edible: false, luxury: false },
    // Gold — minted currency, not a perishable: it occupies Vault space but
    // never decays, and values 1:1 (a unit of Gold IS a unit of value).
    ResourceSpec { name: "Gold", category: Category::Manufactured, tier: 4, spoil_rate: 0.0, bulk: 0.02, pool: StorageClass::Other, edible: false, luxury: false },

    // --- Luxury Goods (tier 5) ---
    ResourceSpec { name: "Wine", category: Category::Luxury, tier: 5, spoil_rate: 0.02, bulk: 1.0, pool: StorageClass::Other, edible: true, luxury: true },
    ResourceSpec { name: "Beer", category: Category::Luxury, tier: 5, spoil_rate: 0.10, bulk: 1.2, pool: StorageClass::Other, edible: true, luxury: true },
    ResourceSpec { name: "Jewelry", category: Category::Luxury, tier: 5, spoil_rate: 0.0, bulk: 0.1, pool: StorageClass::Other, edible: false, luxury: true },
    ResourceSpec { name: "Furniture", category: Category::Luxury, tier: 5, spoil_rate: 0.0, bulk: 3.0, pool: StorageClass::Other, edible: false, luxury: true },
    ResourceSpec { name: "Fine Clothes", category: Category::Luxury, tier: 5, spoil_rate: 0.0, bulk: 0.8, pool: StorageClass::Other, edible: false, luxury: true },
    ResourceSpec { name: "Books", category: Category::Luxury, tier: 5, spoil_rate: 0.0, bulk: 0.5, pool: StorageClass::Other, edible: false, luxury: true },
    ResourceSpec { name: "Candles", category: Category::Luxury, tier: 5, spoil_rate: 0.03, bulk: 0.4, pool: StorageClass::Other, edible: false, luxury: true },
];

/// Look up a resource's registry spec by name (`RESOURCES.get`), `None` for
/// an unknown name — the port's analogue of Python's dict defaulting.
pub fn spec(name: &str) -> Option<&'static ResourceSpec> {
    RESOURCES.iter().find(|r| r.name == name)
}

/// Port of `resource_value`: gold-equivalent value of `amount` units, from
/// the tier table. Gold itself is the one special case — a unit of Gold IS
/// a unit of gold-equivalent value, 1:1. Unknown resources price at tier 3
/// like the Python default.
pub fn resource_value(name: &str, amount: f64) -> f64 {
    if name == "Gold" {
        return amount;
    }
    let tier = spec(name).map(|s| s.tier as usize).unwrap_or(3);
    amount * BASE_VALUE_BY_TIER[tier]
}

/// Python `BASE_VALUE_BY_TIER` — gold/unit before scarcity, tier-indexed.
pub const BASE_VALUE_BY_TIER: [f64; 6] = [0.0, 2.0, 3.0, 5.0, 9.0, 15.0];

/// Convenience accessors — every unknown-name case defaults exactly like the
/// Python dict `.get(name, default)` calls they replace.
pub fn spoil_rate(name: &str) -> f64 {
    spec(name).map(|s| s.spoil_rate).unwrap_or(0.0)
}
pub fn bulk(name: &str) -> f64 {
    spec(name).map(|s| s.bulk).unwrap_or(1.0)
}
pub fn pool(name: &str) -> StorageClass {
    spec(name).map(|s| s.pool).unwrap_or(StorageClass::Other)
}
pub fn is_edible(name: &str) -> bool {
    spec(name).map(|s| s.edible).unwrap_or(false)
}
pub fn is_luxury(name: &str) -> bool {
    spec(name).map(|s| s.luxury).unwrap_or(false)
}
pub fn category_name(name: &str) -> &'static str {
    spec(name).map(|s| s.category.name()).unwrap_or("Unknown")
}

// --- consumption pools -------------------------------------------------------
// The Python game's `_FOOD_SOURCES` / `_TIMBER_SOURCES` / `_LUXURY_GOODS`:
// category-filtered lists of registry names. Only the members a surface town
// can actually hold in M3 are listed here (Fish/Subterranean/Food-Product
// extras arrive with their own production milestones); every listed member
// is included by the same registry rule as the Python originals, which the
// reconciliation test below pins down.

/// Edible Food Products + edible raw Crops + Fish + the edible Subterranean
/// foods (the Python `_FOOD_SOURCES` pool, category-filtered: Cotton and
/// Fodder are `edible: False`, and Salt/Wine/Beer — though flagged edible as
/// "consumed by mouth" — are deliberately not staples a population can live
/// on, exactly the Python note). The members whose raw inputs no M3 system
/// produces (Meat, Fish, …) simply never appear in stock; listing them keeps
/// the pool byte-for-byte the Python original, which the reconciliation test
/// pins down.
pub const FOOD_SOURCES: &[&str] = &[
    // edible Crops
    "Wheat", "Barley", "Oats", "Rye", "Potatoes", "Carrots", "Onions",
    "Beans", "Peas", "Rice", "Grapes",
    // edible Food Products
    "Flour", "Bread", "Meat", "Milk", "Cheese", "Eggs", "Honey",
    "Smoked Fish", "Salted Meat",
    // Fishing and the edible Subterranean foods
    "Fish", "Mushrooms", "Cave Fish",
];

/// Timber upkeep sources (`_TIMBER_SOURCES`): raw wood or the milled form.
pub const TIMBER_SOURCES: &[&str] = &["Planks", "Logs"];

/// Every Luxury Good is interchangeable for satisfying luxury demand
/// (`_LUXURY_GOODS`).
pub const LUXURY_GOODS: &[&str] = &[
    "Wine", "Beer", "Jewelry", "Furniture", "Fine Clothes", "Books", "Candles",
];

// --- conversion (RECIPES, ported continuously) -------------------------------

/// Max units of output a single recipe can produce per day
/// (`CONVERSION_RATE_CAP` — the Python constant is per turn; here it is the
/// continuous per-day rate the fixed timestep integrates).
pub const CONVERSION_RATE_CAP: f64 = 30.0;
/// Luxury Goods convert far slower than staple processed goods — they
/// "shouldn't be widespread until industries have been running a while".
pub const LUXURY_CONVERSION_RATE_CAP: f64 = 2.0;
/// A full year (`TURNS_PER_SEASON * 4`) — no Luxury Good converts AT ALL
/// before this, on top of the trickle-rate cap above.
pub const LUXURY_CONVERSION_MIN_DAYS: f64 = 100.0;

/// One processed resource and its alternative recipes (`RECIPES`): a list of
/// input-lists, each alternative self-sufficient (Cloth from Wool *or*
/// Cotton; Leather from any of five Livestock). Inputs within one
/// alternative are all-of (Shields needs Iron AND Hardwood).
pub struct Recipe {
    pub output: &'static str,
    pub alternatives: &'static [&'static [&'static str]],
}

/// The Python `RECIPES` table verbatim, in `resources.py` order. Livestock
/// and Mining inputs stay in the table — faithful to the spec — but nothing
/// produces those raw inputs in M3, so their recipes can never fire (exactly
/// like Python's "available for a livestock source is always 0").
pub const RECIPES: &[Recipe] = &[
    Recipe { output: "Flour", alternatives: &[&["Wheat"]] },            // Mill
    Recipe { output: "Bread", alternatives: &[&["Flour"]] },            // Bakery
    Recipe { output: "Cheese", alternatives: &[&["Milk"]] },            // Creamery
    Recipe { output: "Smoked Fish", alternatives: &[&["Fish"]] },       // Smokehouse
    Recipe { output: "Planks", alternatives: &[&["Logs"]] },            // Sawmill
    Recipe { output: "Bricks", alternatives: &[&["Clay"]] },            // Brickworks
    Recipe { output: "Glass", alternatives: &[&["Sand"]] },             // Glassworks
    Recipe { output: "Cloth", alternatives: &[&["Wool"], &["Cotton"]] }, // Weaver
    Recipe { output: "Clothes", alternatives: &[&["Cloth"]] },          // Tailor
    Recipe { output: "Leather", alternatives: &[&["Cattle"], &["Sheep"], &["Goats"], &["Pigs"], &["Horses"]] }, // Tannery
    Recipe { output: "Tools", alternatives: &[&["Iron"]] },             // Toolsmith
    Recipe { output: "Weapons", alternatives: &[&["Iron", "Softwood"]] }, // Weaponsmith
    Recipe { output: "Shields", alternatives: &[&["Iron", "Hardwood"]] }, // Shieldwright
    Recipe { output: "Paper", alternatives: &[&["Cotton"]] },           // Papermill
    Recipe { output: "Gold", alternatives: &[&["Gold Ore"]] },          // Mint
    Recipe { output: "Wine", alternatives: &[&["Grapes"]] },            // Winery
    Recipe { output: "Beer", alternatives: &[&["Barley"]] },            // Brewery
    Recipe { output: "Jewelry", alternatives: &[&["Gems"]] },           // Jeweler
    Recipe { output: "Furniture", alternatives: &[&["Planks"]] },       // Furniture Maker
    Recipe { output: "Fine Clothes", alternatives: &[&["Cloth"]] },     // Dressmaker
    Recipe { output: "Books", alternatives: &[&["Paper", "Leather"]] }, // Bindery
    Recipe { output: "Candles", alternatives: &[&["Honey"]] },          // Chandler
];

/// Per-day conversion amounts for the current stock, in `RECIPES` order:
/// what each recipe would convert right now without mutating anything.
/// Mirrors `advance_settlement_production_chains`: first alternative with
/// any stock at all wins, the scarcest input binds, output is capped at
/// `cap`/day, and Luxury Goods stay shut until `LUXURY_CONVERSION_MIN_DAYS`.
/// Used for the HUD; `convert` applies the same plan mutating.
pub fn conversion_rates(res: &HashMap<String, f64>, seconds: f64) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for recipe in RECIPES {
        let cap = if is_luxury(recipe.output) {
            if seconds < LUXURY_CONVERSION_MIN_DAYS {
                continue;
            }
            LUXURY_CONVERSION_RATE_CAP
        } else {
            CONVERSION_RATE_CAP
        };
        for inputs in recipe.alternatives {
            let available = inputs
                .iter()
                .map(|i| res.get(*i).copied().unwrap_or(0.0))
                .fold(f64::INFINITY, f64::min);
            let amount = available.min(cap);
            if amount > 0.0 {
                out.push((recipe.output.to_string(), amount));
                break;
            }
        }
    }
    out
}

/// Apply one tick of conversion: `conversion_rates` × `dt`, consuming the
/// inputs and banking the outputs (1:1). Deterministic — same stock, same
/// order, same result, so the fingerprint tests hold.
pub fn convert(res: &mut HashMap<String, f64>, seconds: f64, dt: f64) {
    if dt <= 0.0 {
        return;
    }
    for recipe in RECIPES {
        let cap = if is_luxury(recipe.output) {
            if seconds < LUXURY_CONVERSION_MIN_DAYS {
                continue;
            }
            LUXURY_CONVERSION_RATE_CAP
        } else {
            CONVERSION_RATE_CAP
        };
        for inputs in recipe.alternatives {
            let available = inputs
                .iter()
                .map(|i| res.get(*i).copied().unwrap_or(0.0))
                .fold(f64::INFINITY, f64::min);
            let amount = available.min(cap) * dt;
            if amount <= 0.0 {
                continue;
            }
            for input in *inputs {
                if let Some(v) = res.get_mut(*input) {
                    *v -= amount;
                }
            }
            *res.entry(recipe.output.to_string()).or_insert(0.0) += amount;
            break;
        }
    }
}

// --- storage (town pools + production throttle) ------------------------------

/// Fraction of a pool's capacity at which primary production starts
/// tapering, and the floor it tapers to at/over capacity
/// (`STORAGE_THROTTLE_START` / `STORAGE_THROTTLE_FLOOR`).
pub const STORAGE_THROTTLE_START: f64 = 0.85;
pub const STORAGE_THROTTLE_FLOOR: f64 = 0.15;

/// The four pools' base capacities for a Town (`STORAGE_POOL_BASE["town"]`):
/// household 840, durable 750, other 150, feed 200. No storage buildings
/// exist in M3 (that is the M4 build milestone), so these are flat.
pub fn pool_capacity(pool: StorageClass) -> f64 {
    match pool {
        StorageClass::Household => 840.0,
        StorageClass::Durable => 750.0,
        StorageClass::Other => 150.0,
        StorageClass::Feed => 200.0,
    }
}

/// Space occupied in one pool — units weighted by bulk (Phase 2 of the
/// Python storage rework), the number every capacity check compares against.
///
/// Sums over the fixed `RESOURCES` table order, never `HashMap` iteration
/// order: Rust's std `HashMap` hashes randomly per map, so a map-ordered
/// float sum is nondeterministic run-to-run — and determinism is load-bearing
/// (the sim must be a pure function of seed + clock + inputs). The Python
/// original iterates the node's dict; the fixed registry order is the port's
/// adaptation with identical results.
pub fn pool_stock(res: &HashMap<String, f64>, p: StorageClass) -> f64 {
    let mut total = 0.0;
    for r in RESOURCES {
        if r.pool == p {
            if let Some(&v) = res.get(r.name) {
                if v > 0.0 {
                    total += v * r.bulk;
                }
            }
        }
    }
    total
}

/// 0..1 multiplier on primary production into `pool`, from how full the
/// pool already is: full rate up to `STORAGE_THROTTLE_START`, then a linear
/// taper to `STORAGE_THROTTLE_FLOOR` at capacity. Port of
/// `storage_throttle` (the feedback loop that made storage buy real output
/// instead of a higher parking level).
pub fn storage_throttle(res: &HashMap<String, f64>, pool: StorageClass) -> f64 {
    let capacity = pool_capacity(pool);
    if capacity <= 0.0 {
        return 1.0;
    }
    let fill = pool_stock(res, pool) / capacity;
    if fill <= STORAGE_THROTTLE_START {
        1.0
    } else if fill >= 1.0 {
        STORAGE_THROTTLE_FLOOR
    } else {
        let span = 1.0 - STORAGE_THROTTLE_START;
        STORAGE_THROTTLE_FLOOR + (1.0 - STORAGE_THROTTLE_FLOOR) * (1.0 - fill) / span
    }
}

// --- spoilage (per-resource + per-pool overflow) -----------------------------

/// Extra decay speed applied while a pool is over capacity
/// (`OVERFLOW_SPOILAGE_MULTIPLIER`), the floor that rate never drops below
/// (`OVERFLOW_MIN_RATE`), and the cap that keeps even the worst case from a
/// full wipeout (`MAX_OVERFLOW_LOSS_FRACTION`).
pub const OVERFLOW_SPOILAGE_MULTIPLIER: f64 = 5.0;
pub const OVERFLOW_MIN_RATE: f64 = 0.10;
pub const MAX_OVERFLOW_LOSS_FRACTION: f64 = 0.75;

/// Port of `_apply_settlement_spoilage_and_overflow`, continuous: every
/// resource decays at its registry `spoil_rate` (rate × dt), then each pool
/// judged independently — a pool packed past capacity decays its overage on
/// top of that, tapering as the overage shrinks instead of an instant
/// cutoff. Gold occupies Vault space but never decays. Tiny remnants are
/// pruned (the Python floors with `int()`, which is the same spirit).
pub fn spoil_and_overflow(res: &mut HashMap<String, f64>, dt: f64) {
    if dt <= 0.0 {
        return;
    }
    for (name, stock) in res.iter_mut() {
        let rate = spoil_rate(name);
        if rate > 0.0 {
            *stock *= 1.0 - rate * dt;
        }
    }
    for p in [
        StorageClass::Household,
        StorageClass::Durable,
        StorageClass::Other,
        StorageClass::Feed,
    ] {
        let capacity = pool_capacity(p);
        if capacity <= 0.0 {
            continue;
        }
        let total = pool_stock(res, p);
        if total <= capacity {
            continue;
        }
        let overage_frac = (total - capacity) / total;
        let names: Vec<String> = res
            .iter()
            .filter(|(r, v)| **v > 0.0 && pool(r.as_str()) == p && r.as_str() != "Gold")
            .map(|(r, _)| r.as_str().to_string())
            .collect();
        for name in &names {
            let stock = res[name];
            let base_rate = spoil_rate(name);
            let overflow_rate = OVERFLOW_MIN_RATE.max(base_rate * OVERFLOW_SPOILAGE_MULTIPLIER);
            let daily_frac = MAX_OVERFLOW_LOSS_FRACTION.min(overflow_rate * overage_frac);
            let loss = stock * daily_frac * dt;
            if let Some(v) = res.get_mut(name) {
                *v -= loss;
            }
        }
    }
    res.retain(|_, v| *v > 1e-9);
}

// --- tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_complete_and_consistent() {
        // Every table row is a valid name, and names are unique.
        let mut seen = std::collections::HashSet::new();
        for r in RESOURCES {
            assert!(seen.insert(r.name), "duplicate registry name {}", r.name);
            assert!(r.tier >= 1 && r.tier <= 5, "tier out of range: {}", r.name);
            assert!(
                (0.0..=1.0).contains(&r.spoil_rate),
                "spoil_rate out of range: {}",
                r.name
            );
            assert!(r.bulk > 0.0, "non-positive bulk: {}", r.name);
        }
        // Category tiers match the Python table's rule ("no resource anywhere
        // whose tier doesn't match its category").
        for r in RESOURCES {
            let expected = match r.category {
                Category::Crops
                | Category::Livestock
                | Category::Fishing
                | Category::Subterranean => 1,
                Category::Forestry | Category::Mining => 2,
                Category::FoodProducts => 3,
                Category::Manufactured => 4,
                Category::Luxury => 5,
            };
            assert_eq!(r.tier, expected, "tier mismatch for {}", r.name);
        }
        // Luxury flag == Luxury category, and every luxury good is pool Other.
        for r in RESOURCES {
            assert_eq!(r.luxury, r.category == Category::Luxury, "luxury flag on {}", r.name);
            if r.luxury {
                assert_eq!(r.pool, StorageClass::Other);
            }
        }
    }

    #[test]
    fn consumption_pools_match_the_registry() {
        // _FOOD_SOURCES = category-filtered staples: edible Crops, Food
        // Products, Fishing and Subterranean members (Salt/Wine/Beer are
        // edible-but-not-staples and excluded, exactly the Python note).
        for name in FOOD_SOURCES {
            let s = spec(name).expect("pool member must be in the registry");
            assert!(
                s.edible
                    && matches!(
                        s.category,
                        Category::Crops
                            | Category::FoodProducts
                            | Category::Fishing
                            | Category::Subterranean
                    ),
                "{} should not be a food source",
                name
            );
        }
        // No eligible staple is missing from the pool.
        let food_from_registry: Vec<&str> = RESOURCES
            .iter()
            .filter(|s| {
                s.edible
                    && matches!(
                        s.category,
                        Category::Crops
                            | Category::FoodProducts
                            | Category::Fishing
                            | Category::Subterranean
                    )
            })
            .map(|s| s.name)
            .collect();
        assert_eq!(food_from_registry.len(), FOOD_SOURCES.len());
        for name in food_from_registry {
            assert!(FOOD_SOURCES.contains(&name), "food pool missing {}", name);
        }
        // Timber sources exist in the registry and are durable.
        for name in TIMBER_SOURCES {
            let s = spec(name).expect("timber source must be in the registry");
            assert_eq!(s.pool, StorageClass::Durable);
        }
        // Every luxury good is flagged luxury in the registry.
        for name in LUXURY_GOODS {
            assert!(is_luxury(name), "{} must be flagged luxury", name);
        }
    }

    #[test]
    fn recipes_reference_only_registry_names() {
        for recipe in RECIPES {
            assert!(spec(recipe.output).is_some(), "recipe output {} unknown", recipe.output);
            for inputs in recipe.alternatives {
                for input in *inputs {
                    assert!(spec(input).is_some(), "recipe input {} unknown", input);
                }
            }
        }
    }

    #[test]
    fn conversion_chain_wheat_to_bread() {
        let mut res = HashMap::new();
        res.insert("Wheat".to_string(), 100.0);
        // One day at a time (dt = 1.0) with a running clock.
        let mut seconds = 0.0;
        for _ in 0..2 {
            convert(&mut res, seconds, 1.0);
            seconds += 1.0;
        }
        // Wheat → Flour → Bread, each at 30/day, 1:1, both chains live in
        // the same pass (Bread converts the Flour the Mill just banked —
        // exactly Python's insertion-ordered RECIPES iteration).
        assert!((res["Flour"] - 0.0).abs() < 1e-9, "Flour should be fully consumed: {:?}", res);
        assert!((res["Bread"] - 60.0).abs() < 1e-9, "Bread should be 60: {:?}", res);
        assert!((res["Wheat"] - 40.0).abs() < 1e-9, "Wheat should be 40: {:?}", res);
    }

    #[test]
    fn luxury_conversion_waits_a_year_and_trickles() {
        let mut res = HashMap::new();
        res.insert("Grapes".to_string(), 500.0);
        // Day 99: no conversion at all yet.
        convert(&mut res, 99.0, 1.0);
        assert_eq!(res["Grapes"], 500.0);
        // Day 100+: 2/day, not the 30/day staple cap.
        convert(&mut res, 100.0, 1.0);
        assert!((res["Wine"] - 2.0).abs() < 1e-9, "wine should trickle at 2/day: {:?}", res);
    }

    #[test]
    fn first_alternative_wins() {
        // Cloth converts from Wool if any wool exists, else Cotton — and
        // only ever one alternative per pass. Clothes then converts the
        // freshly-banked Cloth, and Paper (rag paper, later in RECIPES
        // order) consumes the Cotton the Cloth recipe passed over.
        let mut res = HashMap::new();
        res.insert("Wool".to_string(), 10.0);
        res.insert("Cotton".to_string(), 10.0);
        convert(&mut res, 0.0, 1.0);
        assert_eq!(res["Wool"], 0.0, "wool should be consumed first");
        assert_eq!(res["Cotton"], 0.0, "cotton goes to the papermill, not the loom");
        assert_eq!(res["Cloth"], 0.0, "cloth feeds the tailor immediately");
        assert!((res["Clothes"] - 10.0).abs() < 1e-9);
        assert!((res["Paper"] - 10.0).abs() < 1e-9);
    }

    #[test]
    fn spoilage_decays_perishables_only() {
        let mut res = HashMap::new();
        res.insert("Wheat".to_string(), 100.0); // 0.03/day
        res.insert("Logs".to_string(), 100.0); // 0.0
        spoil_and_overflow(&mut res, 1.0);
        assert!((res["Wheat"] - 97.0).abs() < 1e-9, "wheat should lose 3%: {:?}", res);
        assert_eq!(res["Logs"], 100.0, "logs never spoil");
    }

    #[test]
    fn overflow_decays_a_full_pool() {
        // Fill the household pool (840) far past capacity with Wheat: the
        // overage must decay on top of ordinary spoilage. The durable pool
        // stays under its 750 cap (200 Logs × 3.0 bulk = 600 space), so its
        // goods are untouched by overflow.
        let mut res = HashMap::new();
        res.insert("Wheat".to_string(), 2000.0);
        res.insert("Logs".to_string(), 200.0);
        spoil_and_overflow(&mut res, 1.0);
        // 2000 wheat: spoilage first (2000*0.97 = 1940), then overflow at
        // min(0.75, 0.03*5 * overage_frac). overage_frac = (1940-840)/1940
        // ≈ 0.567 → overflow_rate 0.15 → daily 0.085 → ~165 lost.
        let wheat = res["Wheat"];
        assert!(wheat < 1940.0, "overflow must decay the overage: {}", wheat);
        assert!(wheat > 1700.0, "decay must be bounded by MAX_OVERFLOW_LOSS_FRACTION: {}", wheat);
        // Logs are under capacity in their own pool → untouched by overflow.
        assert_eq!(res["Logs"], 200.0);
    }

    #[test]
    fn gold_never_decays_even_when_overflowing() {
        let mut res = HashMap::new();
        res.insert("Gold".to_string(), 5000.0); // vault cap 150, bulk 0.02 → 100 space… still
        res.insert("Wine".to_string(), 100.0); // forces the Other pool over 150 (100*1.0 + 100)
        // Recompute: Other pool space = 5000*0.02 + 100*1.0 = 200 > 150.
        spoil_and_overflow(&mut res, 1.0);
        assert_eq!(res["Gold"], 5000.0, "Gold never decays");
        assert!(res["Wine"] < 100.0, "Wine in the overflowing vault decays");
    }

    #[test]
    fn storage_throttle_tapers_by_fill() {
        let empty = HashMap::new();
        assert_eq!(storage_throttle(&empty, StorageClass::Household), 1.0);

        let mut res = HashMap::new();
        // At the taper start (85% full) production still runs at full rate…
        res.insert("Wheat".to_string(), 840.0 * STORAGE_THROTTLE_START);
        assert_eq!(storage_throttle(&res, StorageClass::Household), 1.0);
        // …half-way through the taper (92.5% full) it sits at the midpoint
        // between 1.0 and the floor…
        res.insert("Wheat".to_string(), 840.0 * 0.925);
        let mid = storage_throttle(&res, StorageClass::Household);
        assert!((mid - 0.575).abs() < 1e-9, "linear midpoint expected, got {}", mid);
        // …and at capacity it clamps to the floor.
        res.insert("Wheat".to_string(), 840.0);
        assert!(
            (storage_throttle(&res, StorageClass::Household) - STORAGE_THROTTLE_FLOOR).abs() < 1e-9
        );
    }

    #[test]
    fn continuous_sim_is_deterministic() {
        // Conversion + spoilage both flow through HashMap iteration; same
        // seed must produce identical results every run (fingerprint).
        let run = || {
            let mut res = HashMap::new();
            for (i, name) in ["Wheat", "Cotton", "Logs", "Grapes", "Barley"].iter().enumerate() {
                res.insert(name.to_string(), 300.0 + i as f64 * 10.0);
            }
            let mut seconds = 0.0;
            for _ in 0..200 {
                convert(&mut res, seconds, 0.1);
                seconds += 0.1;
                spoil_and_overflow(&mut res, 0.1);
            }
            let mut v: Vec<(String, f64)> = res.into_iter().collect();
            v.sort_by(|a, b| a.0.cmp(&b.0));
            v
        };
        assert_eq!(run(), run());
    }
}
