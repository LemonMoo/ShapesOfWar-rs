//! `war` — M5a: units, movement, mustering.
//!
//! The first slice of the war milestone. What exists here:
//!
//!   * the **unit registry** — all ten archetypes and the five species
//!     commander profiles, ported from `app/battle/unit_types.py` as pure
//!     data (add a type here and it is immediately usable by any army);
//!   * **continuous movement** — units integrate `speed × dt` in cell space,
//!     steered by the same seek + separation + avoidance as
//!     `app/battle/movement.py`: no pathfinding, capped deflection, no
//!     steering for units already in contact (M5b);
//!   * **mustering** — the levy pipeline from `app/world/resources.py`
//!     (`_recompute_military`): levy = adults × MOBILIZATION_RATE,
//!     armed/militia split, shield and cavalry bonuses, floor and ceiling.
//!
//! What does *not* exist yet (M5b/M5c): combat resolution (take_hit,
//! cooldown firing, the charge state machine, auras read on demand),
//! target selection, command AI, caravan raids, blockades.
//!
//! **Time mapping.** The Python battle runs in real-time seconds on a
//! canvas; the sim runs in days. A battle is a *sub-clock* of the sim:
//! 1 sim-second (1 day) = `BATTLE_TIME_SCALE` (60) battle-seconds, so a
//! 60-second battle takes one in-game day — a battle is an event, not a
//! season. `Battle::sim_tick` takes sim-seconds and does the scaling.
//!
//! **Space mapping.** Python positions are canvas pixels; here they are
//! grid cells (f64). `PIXELS_PER_CELL = 8` converts every ported pixel
//! constant (speeds, ranges, steering distances) into cell units, and the
//! battle field is `BATTLE_FIELD_CELLS`² — 100×100 cells, the same order
//! as the Python 800px canvas.

use std::collections::HashMap;

use crate::settlement::Settlement;

/// 1 sim-second (1 day) = this many battle-seconds.
pub const BATTLE_TIME_SCALE: f64 = 60.0;
/// Canvas pixels per grid cell — the px→cell conversion for ported constants.
pub const PIXELS_PER_CELL: f64 = 8.0;
/// Side of the battle field in cells (≈ the Python 800px canvas).
pub const BATTLE_FIELD_CELLS: f64 = 100.0;
/// A unit this close to its move point has arrived.
const ARRIVE_EPS: f64 = 0.05;

// --- steering constants (movement.py, px → cells) ---------------------------
const PX: f64 = 1.0 / PIXELS_PER_CELL;
/// Personal space between allies, measured on top of the two body radii (9px).
pub const PERSONAL_SPACE: f64 = 9.0 * PX;
pub const SEPARATION_WEIGHT: f64 = 1.15;
/// How far ahead an ally counts as being in the way (30px).
pub const AVOID_DIST: f64 = 30.0 * PX;
/// How narrow "ahead" is — cos of ~35° off the direction of travel.
pub const AVOID_CONE_COS: f64 = 0.82;
pub const AVOID_WEIGHT: f64 = 1.30;
/// Steering may not turn a unit more than this far off its objective
/// (cos 60°) — an army that circles is worse than an army that clumps.
pub const MAX_DEFLECT_COS: f64 = 0.5;
/// How many allies may be in contact with one enemy before the rest hold
/// (M5b reads this off the contact snapshot).
pub const CONTACT_CAP: usize = 3;
/// A unit held off a mobbed enemy still closes this far (16px) — the second
/// rank, right behind the fighting (M5b).
pub const SWARM_STANDOFF: f64 = 16.0 * PX;
/// Infiltrators swing wide of *enemy* bodies out to this far (52px) — wider
/// than the ally cone, because going around a formation means committing to
/// the detour early.
pub const INFILTRATE_AVOID_DIST: f64 = 52.0 * PX;
pub const INFILTRATE_WEIGHT: f64 = 1.9;
/// Spatial grid cell for the per-tick neighbour lookup (32px, battle.py
/// `MOVE_CELL`).
pub const MOVE_CELL: f64 = 32.0 * PX;

// --- commander constants (unit_types.py) -------------------------------------
/// Commander aura radius, canvas px (`COMMANDER_AURA_RADIUS`).
pub const COMMANDER_AURA_RADIUS: f64 = 130.0;
/// A commander past this far (px) from where his soldiers are fighting stops
/// everything else and comes back.
pub const COMMANDER_LEASH: f64 = 165.0;
/// Once his own army is down to this many soldiers or fewer, a commander
/// fights like anyone else.
pub const COMMANDER_LAST_STAND: usize = 3;
/// He rides back, rather than trudging after an army that gallops.
pub const COMMANDER_RETURN_SPEED_MULT: f64 = 1.6;

// --- mustering (resources.py `_recompute_military`) --------------------------
/// Share of a realm's ADULTS it can put in the field.
pub const MOBILIZATION_RATE: f64 = 0.08;
/// A levied adult with no weapon still shows up, at this weight.
pub const MILITIA_WEIGHT: f64 = 0.30;
/// Fully equipping your armed soldiers with shields is worth this much.
pub const SHIELD_BONUS: f64 = 0.25;
/// Fully mounting your armed soldiers is worth this much.
pub const CAVALRY_BONUS: f64 = 0.50;
/// A realm always fields at least this much strength.
pub const MILITARY_FLOOR: f64 = 10.0;
/// ...and never more than this.
pub const MILITARY_CEILING: f64 = 1200.0;

/// The five playable species (the `COMMANDER_BY_SPECIES` keys).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Species {
    Humans,
    Elves,
    Dwarves,
    Orcs,
    Goblins,
}

impl Species {
    pub fn key(self) -> &'static str {
        match self {
            Species::Humans => "Humans",
            Species::Elves => "Elves",
            Species::Dwarves => "Dwarves",
            Species::Orcs => "Orcs",
            Species::Goblins => "Goblins",
        }
    }
}

/// The ten archetypes (the `UNIT_TYPES` keys).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnitKind {
    Infantry,
    Cavalry,
    Commander,
    Archer,
    Assassin,
    Sapper,
    Shieldwarden,
    Berserker,
    Bladesinger,
    Bannerman,
}

impl UnitKind {
    pub fn key(self) -> &'static str {
        match self {
            UnitKind::Infantry => "infantry",
            UnitKind::Cavalry => "cavalry",
            UnitKind::Commander => "commander",
            UnitKind::Archer => "archer",
            UnitKind::Assassin => "assassin",
            UnitKind::Sapper => "sapper",
            UnitKind::Shieldwarden => "shieldwarden",
            UnitKind::Berserker => "berserker",
            UnitKind::Bladesinger => "bladesinger",
            UnitKind::Bannerman => "bannerman",
        }
    }

    pub fn from_key(key: &str) -> Option<UnitKind> {
        (0..=9u8)
            .map(|i| unsafe { std::mem::transmute::<u8, UnitKind>(i) })
            .find(|k| k.key() == key)
    }
}

/// A commander-style aura. Auras do NOT stack: a soldier takes the best
/// single source covering it, read on demand at the point of use (M5b)
/// rather than written onto units, so it can never double-apply and needs
/// no cleanup when the source dies.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Aura {
    pub damage_mult: f64,
    pub cooldown_mult: f64,
    pub block_add: f64,
    pub dodge_add: f64,
    pub range_mult: f64,
    pub damage_taken_mult: f64,
}

impl Aura {
    /// The no-effect values (mults 1.0, adds 0.0).
    pub fn identity() -> Self {
        Aura {
            damage_mult: 1.0,
            cooldown_mult: 1.0,
            block_add: 0.0,
            dodge_add: 0.0,
            range_mult: 1.0,
            damage_taken_mult: 1.0,
        }
    }
}

/// One archetype's full stat block (a `UNIT_TYPES[key]` entry).
/// Speeds/ranges are canvas px; times are battle-seconds.
#[derive(Clone, Copy, Debug)]
pub struct UnitStats {
    pub name: &'static str,
    /// Body radius, canvas px (steering personal space is measured on top).
    pub radius: f64,
    pub max_hp: f64,
    /// Speed, canvas px per battle-second.
    pub speed: f64,
    /// Attack range, canvas px.
    pub range: f64,
    pub damage: f64,
    /// Seconds between attacks.
    pub cooldown: f64,
    /// Fires an arrow at the target on each hit.
    pub ranged: bool,
    /// Chance (0..1) an in-range, off-cooldown attack lands; 1.0 when the
    /// key is omitted in Python.
    pub accuracy: f64,
    /// Chance to deflect an incoming hit with a shield — but only if the
    /// attacker is within the frontal `block_arc_deg` cone.
    pub block_chance: f64,
    /// Total width (degrees) of the frontal shield cone.
    pub block_arc_deg: f64,
    /// Mounted: builds momentum while galloping toward its target.
    pub charge: bool,
    /// Extra damage multiplier at full momentum (added on `melee_floor`).
    pub charge_bonus: f64,
    /// Damage multiplier with zero momentum (stuck in a scrum) — below 1.0,
    /// so a bogged-down rider hits softer than its raw damage implies.
    pub melee_floor: f64,
    /// Seconds of uninterrupted galloping to reach full momentum.
    pub charge_ramp: f64,
    /// A couched impact ploughs into whatever frontline it reaches: everyone
    /// else within this radius of the struck target takes `charge_aoe_share`.
    pub charge_aoe_radius: f64,
    pub charge_aoe_share: f64,
    /// Floor on this type's dodge chance, taken as a MAX against the owning
    /// species' own dodge.
    pub dodge_chance: f64,
    /// Damage multiplier on the opening blow against each DISTINCT target.
    pub first_strike: f64,
    /// Ignores everything but enemy ranged units while any are alive.
    pub hunts_ranged: bool,
    /// Extra damage multiplier at ZERO hp, scaled by how much health is
    /// already gone — more dangerous hurt than whole.
    pub frenzy: f64,
    /// A landed hit also deals `splash_share` of its damage to every other
    /// enemy within this radius of the target.
    pub splash_radius: f64,
    pub splash_share: f64,
    /// What the unit DOES on the field (roles.py): "infiltrator", "bombard",
    /// "anchor", "frenzied", "flanker", "banner".
    pub role: Option<&'static str>,
    /// Ignores the formation spacing everyone else keeps (movement.steer).
    pub no_cohesion: bool,
    /// Commander-style aura projected over friendly soldiers.
    pub aura: Option<Aura>,
    /// Aura radius, canvas px.
    pub aura_radius: f64,
}

impl UnitStats {
    fn base(
        name: &'static str,
        radius: f64,
        max_hp: f64,
        speed: f64,
        range: f64,
        damage: f64,
        cooldown: f64,
    ) -> Self {
        UnitStats {
            name,
            radius,
            max_hp,
            speed,
            range,
            damage,
            cooldown,
            ranged: false,
            accuracy: 1.0,
            block_chance: 0.0,
            block_arc_deg: 0.0,
            charge: false,
            charge_bonus: 0.0,
            melee_floor: 1.0,
            charge_ramp: 0.0,
            charge_aoe_radius: 0.0,
            charge_aoe_share: 0.0,
            dodge_chance: 0.0,
            first_strike: 1.0,
            hunts_ranged: false,
            frenzy: 0.0,
            splash_radius: 0.0,
            splash_share: 0.0,
            role: None,
            no_cohesion: false,
            aura: None,
            aura_radius: 0.0,
        }
    }
}

/// The registry (Python `UNIT_TYPES`).
pub fn stats_for(kind: UnitKind) -> UnitStats {
    match kind {
        UnitKind::Infantry => {
            let mut s = UnitStats::base("Swordsmen", 5.0, 30.0, 34.0, 14.0, 8.0, 0.6);
            s.block_chance = 0.35;
            s.block_arc_deg = 150.0;
            s
        }
        UnitKind::Cavalry => {
            let mut s = UnitStats::base("Cavalry", 6.0, 26.0, 110.0, 12.0, 10.0, 0.5);
            s.charge = true;
            s.charge_bonus = 3.0;
            s.melee_floor = 0.5;
            s.charge_ramp = 1.0;
            s.charge_aoe_radius = 22.0;
            s.charge_aoe_share = 0.35;
            s
        }
        UnitKind::Commander => {
            let mut s = UnitStats::base("Commander", 15.0, 270.0, 30.0, 18.0, 24.0, 0.7);
            s.block_chance = 0.45;
            s.block_arc_deg = 180.0;
            s
        }
        UnitKind::Archer => {
            let mut s = UnitStats::base("Archer", 5.0, 20.0, 30.0, 180.0, 5.5, 0.9);
            s.ranged = true;
            s.accuracy = 0.60;
            s
        }
        UnitKind::Assassin => {
            let mut s = UnitStats::base("Assassin", 4.0, 14.0, 46.0, 12.0, 4.0, 0.22);
            s.dodge_chance = 0.22;
            s.role = Some("infiltrator");
            s.first_strike = 3.5;
            s.hunts_ranged = true;
            s
        }
        UnitKind::Sapper => {
            let mut s = UnitStats::base("Sapper", 5.0, 16.0, 40.0, 110.0, 7.0, 2.6);
            s.ranged = true;
            s.accuracy = 0.65;
            s.splash_radius = 26.0;
            s.splash_share = 0.70;
            s.role = Some("bombard");
            s
        }
        UnitKind::Shieldwarden => {
            let mut s = UnitStats::base("Shieldwarden", 6.0, 40.0, 26.0, 14.0, 7.0, 0.90);
            s.block_chance = 0.45;
            s.block_arc_deg = 170.0;
            s.aura = Some(Aura {
                damage_taken_mult: 0.88,
                ..Aura::identity()
            });
            s.aura_radius = 80.0;
            s.role = Some("anchor");
            s
        }
        UnitKind::Berserker => {
            let mut s = UnitStats::base("Berserker", 6.0, 34.0, 40.0, 14.0, 12.0, 0.50);
            s.frenzy = 0.8;
            s.role = Some("frenzied");
            s.no_cohesion = true;
            s
        }
        UnitKind::Bladesinger => {
            let mut s = UnitStats::base("Bladesinger", 5.0, 22.0, 46.0, 13.0, 9.0, 0.45);
            s.dodge_chance = 0.20;
            s.role = Some("flanker");
            s
        }
        UnitKind::Bannerman => {
            let mut s = UnitStats::base("Standard Bearer", 6.0, 34.0, 32.0, 14.0, 6.0, 0.80);
            s.block_chance = 0.35;
            s.block_arc_deg = 150.0;
            s.aura = Some(Aura {
                damage_mult: 1.14,
                block_add: 0.08,
                cooldown_mult: 0.94,
                ..Aura::identity()
            });
            s.aura_radius = 125.0;
            s.role = Some("banner");
            s
        }
    }
}

/// A species' commander (a `COMMANDER_BY_SPECIES` entry). The `*` fields
/// override the base commander entry in the registry; `aura` is applied to
/// living friendly soldiers within `COMMANDER_AURA_RADIUS`.
#[derive(Clone, Copy, Debug)]
pub struct CommanderProfile {
    pub title: &'static str,
    pub max_hp: Option<f64>,
    pub damage: Option<f64>,
    pub range: Option<f64>,
    pub ranged: Option<bool>,
    pub accuracy: Option<f64>,
    pub speed: Option<f64>,
    pub block_chance: Option<f64>,
    pub dodge_chance: Option<f64>,
    /// The Warchief's cleaving arc: (radius px, share).
    pub cleave: Option<(f64, f64)>,
    pub aura: Option<Aura>,
}

/// The species' commander profile (Python `commander_profile`).
pub fn commander_profile(species: Species) -> CommanderProfile {
    match species {
        // The Marshal: least dangerous commander alive, the best force
        // multiplier in the game.
        Species::Humans => CommanderProfile {
            title: "Marshal",
            max_hp: Some(250.0),
            damage: Some(18.0),
            range: None,
            ranged: None,
            accuracy: None,
            speed: None,
            block_chance: None,
            dodge_chance: None,
            cleave: None,
            aura: Some(Aura {
                damage_mult: 1.15,
                cooldown_mult: 0.90,
                block_add: 0.10,
                ..Aura::identity()
            }),
        },
        // The Warden: fights at range and extends his archers' reach.
        Species::Elves => CommanderProfile {
            title: "Warden",
            max_hp: Some(190.0),
            damage: Some(11.0),
            range: Some(150.0),
            ranged: Some(true),
            accuracy: Some(0.9),
            speed: Some(34.0),
            block_chance: Some(0.0),
            dodge_chance: None,
            cleave: None,
            aura: Some(Aura {
                range_mult: 1.05,
                cooldown_mult: 0.95,
                ..Aura::identity()
            }),
        },
        // The Thane: the anchor. Enormous and immovable.
        Species::Dwarves => CommanderProfile {
            title: "Thane",
            max_hp: Some(470.0),
            damage: Some(26.0),
            range: None,
            ranged: None,
            accuracy: None,
            speed: Some(28.0),
            block_chance: Some(0.58),
            dodge_chance: None,
            cleave: None,
            aura: Some(Aura {
                damage_taken_mult: 0.76,
                ..Aura::identity()
            }),
        },
        // The Warchief: pure offence, no army aura beyond raw damage.
        Species::Orcs => CommanderProfile {
            title: "Warchief",
            max_hp: Some(300.0),
            damage: Some(30.0),
            range: None,
            ranged: None,
            accuracy: None,
            speed: None,
            block_chance: Some(0.15),
            dodge_chance: None,
            cleave: Some((34.0, 0.55)),
            aura: Some(Aura {
                damage_mult: 1.10,
                ..Aura::identity()
            }),
        },
        // The Chieftain: never meant to be hit.
        Species::Goblins => CommanderProfile {
            title: "Chieftain",
            max_hp: Some(200.0),
            damage: Some(14.0),
            range: None,
            ranged: None,
            accuracy: None,
            speed: Some(42.0),
            block_chance: None,
            dodge_chance: Some(0.32),
            cleave: None,
            aura: Some(Aura {
                dodge_add: 0.045,
                cooldown_mult: 0.98,
                ..Aura::identity()
            }),
        },
    }
}

/// One soldier on the field. Positions are cell coordinates; `facing` is
/// radians (0 = +x). The `charge` momentum and `cooldown` are carried now
/// but only become meaningful with M5b's combat resolution.
#[derive(Clone, Debug)]
pub struct Unit {
    pub id: u32,
    pub kind: UnitKind,
    /// Side index (M5a: one army per side).
    pub faction: usize,
    pub x: f64,
    pub y: f64,
    pub hp: f64,
    pub facing: f64,
    /// Seconds until the next attack is ready (M5b fires on it).
    pub cooldown: f64,
    /// Cavalry momentum 0..1 (M5b's charge state machine updates it).
    pub charge: f64,
    /// Direct move order — the only order M5a understands (Python
    /// `Unit.move_point`). Targets arrive with M5b.
    pub move_point: Option<(f64, f64)>,
    /// Set on commanders: their species profile overrides the base stats.
    pub species: Option<Species>,
}

impl Unit {
    pub fn alive(&self) -> bool {
        self.hp > 0.0
    }

    /// Effective max hp — the base stat, with the species commander
    /// override applied (Python `Unit.__init__` bakes the profile in).
    pub fn max_hp(&self) -> f64 {
        let mut m = stats_for(self.kind).max_hp;
        if self.kind == UnitKind::Commander {
            if let Some(sp) = self.species {
                if let Some(v) = commander_profile(sp).max_hp {
                    m = v;
                }
            }
        }
        m
    }

    /// Speed in cells per battle-second.
    pub fn speed_cells(&self) -> f64 {
        let mut s = stats_for(self.kind).speed;
        if self.kind == UnitKind::Commander {
            if let Some(sp) = self.species {
                if let Some(v) = commander_profile(sp).speed {
                    s = v;
                }
            }
        }
        s / PIXELS_PER_CELL
    }
}

/// Per-tick spatial index (Python `battle._move_grid`), rebuilt every tick.
/// Buckets are keyed by `(cx, cy)` of `pos / MOVE_CELL` floored — the same
/// floor division Python's `int(x // cell)` does for negatives.
pub struct MoveGrid {
    buckets: HashMap<i64, Vec<usize>>,
}

impl MoveGrid {
    pub fn build(units: &[Unit]) -> Self {
        let mut buckets: HashMap<i64, Vec<usize>> = HashMap::new();
        for (i, u) in units.iter().enumerate() {
            if !u.alive() {
                continue;
            }
            let cx = (u.x / MOVE_CELL).floor() as i32;
            let cy = (u.y / MOVE_CELL).floor() as i32;
            let key = (cx as i64) << 32 | (cy as i64 & 0xFFFF_FFFF);
            buckets.entry(key).or_default().push(i);
        }
        MoveGrid { buckets }
    }

    /// Every living unit in the 3×3 ring of cells around (x, y).
    pub fn ring(&self, x: f64, y: f64) -> Vec<usize> {
        let cx = (x / MOVE_CELL).floor() as i32;
        let cy = (y / MOVE_CELL).floor() as i32;
        let mut out = Vec::new();
        for gx in (cx - 1)..=cx + 1 {
            for gy in (cy - 1)..=cy + 1 {
                let key = (gx as i64) << 32 | (gy as i64 & 0xFFFF_FFFF);
                if let Some(v) = self.buckets.get(&key) {
                    out.extend_from_slice(v);
                }
            }
        }
        out
    }
}

/// Port of `movement.steer`: the direction `unit` should actually walk this
/// tick, given that it wants to go `(dx, dy)` — a unit vector. Returns a
/// unit vector.
///
/// A unit type may opt out with `no_cohesion` (the Berserker, whose whole
/// character is that he does not keep a line). Enemies are obstacles only
/// for an infiltrator, and its own target never counts as one (M5b).
pub fn steer(unit: &Unit, dx: f64, dy: f64, grid: &MoveGrid, units: &[Unit]) -> (f64, f64) {
    let stats = stats_for(unit.kind);
    let infiltrating = stats.role == Some("infiltrator");
    if stats.no_cohesion && !infiltrating {
        return (dx, dy);
    }

    let mut sx = 0.0;
    let mut sy = 0.0; // separation
    let mut ax = 0.0;
    let mut ay = 0.0; // avoidance
    for &j in &grid.ring(unit.x, unit.y) {
        if j == unit.id as usize {
            continue;
        }
        let other = &units[j];
        let is_ally = other.faction == unit.faction;
        // Non-infiltrators never treat enemy bodies as obstacles.
        if !is_ally && !infiltrating {
            continue;
        }
        let ox = other.x - unit.x;
        let oy = other.y - unit.y;
        let d2 = ox * ox + oy * oy;
        if d2 < 1e-9 {
            continue;
        }
        let d = d2.sqrt();
        let space = stats.radius + stats_for(other.kind).radius + PERSONAL_SPACE;
        if is_ally && d < space {
            // Linear falloff: an ally at arm's length barely registers, one
            // standing on top of you dominates. Enemies are never pushed
            // away from — backing off an enemy is retreating, not spacing.
            let push = (space - d) / space;
            sx -= ox / d * push;
            sy -= oy / d * push;
        }
        let reach = if is_ally { AVOID_DIST } else { INFILTRATE_AVOID_DIST };
        if d < reach {
            // Only what is genuinely in front, and only close enough to be
            // an obstacle rather than a distant body in the same direction.
            let ahead = (ox * dx + oy * dy) / d;
            if ahead >= AVOID_CONE_COS {
                // Step to the side the body is NOT on: the sign of the cross
                // product says which side it stands, and the perpendicular
                // away from it is where the gap is.
                let cross = dx * oy - dy * ox;
                let side = if cross > 0.0 { -1.0 } else { 1.0 };
                let mut weight = (reach - d) / reach * ahead;
                if !is_ally {
                    weight *= INFILTRATE_WEIGHT;
                }
                ax += -dy * side * weight;
                ay += dx * side * weight;
            }
        }
    }

    let vx = dx + SEPARATION_WEIGHT * sx + AVOID_WEIGHT * ax;
    let vy = dy + SEPARATION_WEIGHT * sy + AVOID_WEIGHT * ay;
    let mag = (vx * vx + vy * vy).sqrt();
    if mag < 1e-6 {
        return (dx, dy);
    }
    let (mut vx, mut vy) = (vx / mag, vy / mag);

    // Cap the deflection. Past the cap, take the nearest heading that is
    // within it — rotate the desired direction toward the steered one by
    // exactly the maximum, rather than falling back to the raw seek, so a
    // unit that is genuinely boxed in still leans the way the gap is.
    let dot = vx * dx + vy * dy;
    if dot < MAX_DEFLECT_COS {
        let cross = dx * vy - dy * vx;
        let sign = if cross > 0.0 { 1.0 } else { -1.0 };
        let ang = MAX_DEFLECT_COS.clamp(-1.0, 1.0).acos() * sign;
        let (ca, sa) = (ang.cos(), ang.sin());
        (vx, vy) = (dx * ca - dy * sa, dx * sa + dy * ca);
    }
    (vx, vy)
}

/// One battle: the units on the field and the battle sub-clock.
#[derive(Clone, Debug)]
pub struct Battle {
    pub units: Vec<Unit>,
    /// Battle-seconds since the battle began (the sub-clock).
    pub seconds: f64,
}

impl Battle {
    pub fn new() -> Self {
        Battle {
            units: Vec::new(),
            seconds: 0.0,
        }
    }

    /// Add a unit at (x, y) cell coordinates. Returns its id.
    pub fn add(&mut self, kind: UnitKind, faction: usize, x: f64, y: f64) -> u32 {
        let id = self.units.len() as u32;
        let mut u = Unit {
            id,
            kind,
            faction,
            x,
            y,
            hp: 0.0,
            facing: 0.0,
            cooldown: 0.0,
            charge: 0.0,
            move_point: None,
            species: None,
        };
        u.hp = u.max_hp();
        self.units.push(u);
        id
    }

    /// Add a species commander (Python `Battle.deploy` adds exactly one per
    /// side on top of the army, so fielding him never costs a soldier).
    pub fn add_commander(&mut self, species: Species, faction: usize, x: f64, y: f64) -> u32 {
        let id = self.add(UnitKind::Commander, faction, x, y);
        let u = &mut self.units[id as usize];
        u.species = Some(species);
        u.hp = u.max_hp(); // re-bake with the profile override applied
        id
    }

    /// Deploy `kinds` for `faction` in a grid of `cols` columns, centred on
    /// `origin`: unit i sits in column `i % cols`, row `i / cols`, at
    /// `spacing` cells between neighbours. (A single line would run off the
    /// field for a full levy.)
    pub fn deploy_grid(
        &mut self,
        faction: usize,
        kinds: &[UnitKind],
        origin: (f64, f64),
        cols: usize,
        spacing: f64,
    ) {
        if kinds.is_empty() || cols == 0 {
            return;
        }
        let rows = (kinds.len() - 1) / cols + 1;
        for (i, kind) in kinds.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;
            let x = origin.0 + (col as f64 - (cols - 1) as f64 / 2.0) * spacing;
            let y = origin.1 + (row as f64 - (rows - 1) as f64 / 2.0) * spacing;
            self.add(*kind, faction, x, y);
        }
    }

    /// Advance the battle by `dt_sim` sim-seconds (days). The battle
    /// sub-clock moves `dt_sim × BATTLE_TIME_SCALE` battle-seconds.
    ///
    /// Two-phase for determinism and borrow rules: first read every unit's
    /// state and compute its steered heading, then integrate positions.
    pub fn sim_tick(&mut self, dt_sim: f64) {
        let dt = dt_sim * BATTLE_TIME_SCALE;
        if dt <= 0.0 {
            return;
        }
        self.seconds += dt;
        let grid = MoveGrid::build(&self.units);

        // Phase 1: desired + steered heading per unit (read-only).
        let mut steps: Vec<Option<(f64, f64, f64)>> = Vec::with_capacity(self.units.len());
        for i in 0..self.units.len() {
            let u = &self.units[i];
            if !u.alive() {
                steps.push(None);
                continue;
            }
            // M5a has no targets: the only order is a direct move point.
            let Some((tx, ty)) = u.move_point else {
                steps.push(None);
                continue;
            };
            let (dx, dy) = (tx - u.x, ty - u.y);
            let d = (dx * dx + dy * dy).sqrt();
            if d < ARRIVE_EPS {
                steps.push(None);
                continue;
            }
            let (nx, ny) = (dx / d, dy / d);
            let (vx, vy) = steer(u, nx, ny, &grid, &self.units);
            // Never overshoot the point: clamp the step to the remaining
            // distance so a unit arrives instead of orbiting the target.
            let step = (u.speed_cells() * dt).min(d);
            steps.push(Some((vx, vy, step)));
        }

        // Phase 2: integrate.
        for (i, s) in steps.iter().enumerate() {
            let u = &mut self.units[i];
            if !u.alive() {
                continue;
            }
            if let Some((vx, vy, step)) = *s {
                u.x += vx * step;
                u.y += vy * step;
                u.facing = vy.atan2(vx);
            }
            if u.cooldown > 0.0 {
                u.cooldown = (u.cooldown - dt).max(0.0);
            }
        }
    }
}

// --- mustering ---------------------------------------------------------------

/// The levy a realm can field (Python `_recompute_military`, the strength
/// part). `species_mil_pct` is the species' `mil` bonus (e.g. +10),
/// `loyalty_mult` the governance loyalty multiplier.
#[derive(Clone, Copy, Debug)]
pub struct Muster {
    /// adults × MOBILIZATION_RATE.
    pub levy: f64,
    /// min(levy, weapons) — the soldiers who get a weapon.
    pub armed: f64,
    /// levy − armed — they march anyway, at MILITIA_WEIGHT.
    pub militia: f64,
    /// The final strength number (floored/ceiled, bonuses applied).
    pub strength: f64,
}

pub fn muster_strength(
    adults: f64,
    weapons: f64,
    shields: f64,
    horses: f64,
    species_mil_pct: f64,
    loyalty_mult: f64,
) -> Muster {
    let levy = adults * MOBILIZATION_RATE;
    let armed = levy.min(weapons);
    let militia = levy - armed;
    let mut strength = armed + militia * MILITIA_WEIGHT;
    if armed > 0.0 {
        strength *= 1.0 + SHIELD_BONUS * armed.min(shields) / armed;
        strength *= 1.0 + CAVALRY_BONUS * armed.min(horses) / armed;
    }
    strength *= 1.0 + species_mil_pct / 100.0;
    strength *= loyalty_mult;
    strength = strength.clamp(MILITARY_FLOOR, MILITARY_CEILING);
    Muster {
        levy,
        armed,
        militia,
        strength,
    }
}

/// A simple deterministic roster for a levy: a quarter archers, the rest
/// infantry. Species specials and the real composition AI come with M5c.
pub fn army_composition(strength: f64) -> Vec<UnitKind> {
    let n = strength as usize;
    let archers = n / 4;
    let mut v = Vec::with_capacity(n);
    v.extend(std::iter::repeat(UnitKind::Archer).take(archers));
    v.extend(std::iter::repeat(UnitKind::Infantry).take(n - archers));
    v
}

/// Muster a field army from a settlement's stockpile and deploy it on the
/// battle field: faction 0 enters from the west edge, faction 1 from the
/// east (the Python armies deploy ~900px apart; here 80 cells).
pub fn muster_army(settlement: &Settlement, faction: usize, species: Species) -> Battle {
    let weapons = settlement.resources.get("Weapons").copied().unwrap_or(0.0);
    let shields = settlement.resources.get("Shields").copied().unwrap_or(0.0);
    let horses = settlement.resources.get("Horses").copied().unwrap_or(0.0);
    let m = muster_strength(settlement.adults, weapons, shields, horses, 0.0, 1.0);

    let entry_x = if faction == 0 {
        10.0
    } else {
        BATTLE_FIELD_CELLS - 10.0
    };
    let kinds = army_composition(m.strength);
    let mut b = Battle::new();
    b.deploy_grid(faction, &kinds, (entry_x, BATTLE_FIELD_CELLS / 2.0), 10, 2.0);
    b.add_commander(species, faction, entry_x, BATTLE_FIELD_CELLS / 2.0);
    b
}

// --- headless fingerprint tests ----------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_matches_python() {
        // All ten archetypes, with the load-bearing numbers.
        let inf = stats_for(UnitKind::Infantry);
        assert_eq!(inf.name, "Swordsmen");
        assert_eq!((inf.max_hp, inf.speed, inf.range, inf.damage, inf.cooldown), (30.0, 34.0, 14.0, 8.0, 0.6));
        assert_eq!((inf.block_chance, inf.block_arc_deg), (0.35, 150.0));
        assert!(!inf.ranged && inf.accuracy == 1.0 && inf.role.is_none());

        let cav = stats_for(UnitKind::Cavalry);
        assert_eq!((cav.speed, cav.charge_bonus, cav.melee_floor, cav.charge_ramp), (110.0, 3.0, 0.5, 1.0));
        assert!(cav.charge);
        assert_eq!((cav.charge_aoe_radius, cav.charge_aoe_share), (22.0, 0.35));

        let cmd = stats_for(UnitKind::Commander);
        assert_eq!((cmd.max_hp, cmd.damage, cmd.radius), (270.0, 24.0, 15.0));

        let arc = stats_for(UnitKind::Archer);
        assert!(arc.ranged && arc.accuracy == 0.60 && arc.range == 180.0);

        let ass = stats_for(UnitKind::Assassin);
        assert_eq!(ass.role, Some("infiltrator"));
        assert_eq!((ass.dodge_chance, ass.first_strike), (0.22, 3.5));
        assert!(ass.hunts_ranged);

        let sap = stats_for(UnitKind::Sapper);
        assert_eq!(sap.role, Some("bombard"));
        assert_eq!((sap.splash_radius, sap.splash_share), (26.0, 0.70));
        assert!(sap.ranged && sap.accuracy == 0.65);

        let sw = stats_for(UnitKind::Shieldwarden);
        assert_eq!(sw.role, Some("anchor"));
        assert_eq!(sw.aura.unwrap().damage_taken_mult, 0.88);
        assert_eq!(sw.aura_radius, 80.0);

        let ber = stats_for(UnitKind::Berserker);
        assert_eq!(ber.role, Some("frenzied"));
        assert_eq!(ber.frenzy, 0.8);
        assert!(ber.no_cohesion);

        let bl = stats_for(UnitKind::Bladesinger);
        assert_eq!(bl.role, Some("flanker"));
        assert_eq!(bl.dodge_chance, 0.20);

        let ban = stats_for(UnitKind::Bannerman);
        assert_eq!(ban.role, Some("banner"));
        let a = ban.aura.unwrap();
        assert_eq!((a.damage_mult, a.block_add, a.cooldown_mult), (1.14, 0.08, 0.94));
        assert_eq!(ban.aura_radius, 125.0);

        // Key round-trip.
        for k in [
            UnitKind::Infantry,
            UnitKind::Cavalry,
            UnitKind::Commander,
            UnitKind::Archer,
            UnitKind::Assassin,
            UnitKind::Sapper,
            UnitKind::Shieldwarden,
            UnitKind::Berserker,
            UnitKind::Bladesinger,
            UnitKind::Bannerman,
        ] {
            assert_eq!(UnitKind::from_key(k.key()), Some(k));
        }
        assert_eq!(UnitKind::from_key("nope"), None);
    }

    #[test]
    fn commander_profiles_match_python() {
        let m = commander_profile(Species::Humans);
        assert_eq!(m.title, "Marshal");
        assert_eq!((m.max_hp, m.damage), (Some(250.0), Some(18.0)));
        let a = m.aura.unwrap();
        assert_eq!((a.damage_mult, a.cooldown_mult, a.block_add), (1.15, 0.90, 0.10));

        let w = commander_profile(Species::Elves);
        assert_eq!(w.title, "Warden");
        assert_eq!((w.max_hp, w.damage, w.range, w.speed), (Some(190.0), Some(11.0), Some(150.0), Some(34.0)));
        assert_eq!(w.ranged, Some(true));
        assert_eq!(w.accuracy, Some(0.9));
        assert_eq!(w.block_chance, Some(0.0));
        let a = w.aura.unwrap();
        assert_eq!((a.range_mult, a.cooldown_mult), (1.05, 0.95));

        let t = commander_profile(Species::Dwarves);
        assert_eq!(t.title, "Thane");
        assert_eq!((t.max_hp, t.damage, t.speed, t.block_chance), (Some(470.0), Some(26.0), Some(28.0), Some(0.58)));
        assert_eq!(t.aura.unwrap().damage_taken_mult, 0.76);

        let wc = commander_profile(Species::Orcs);
        assert_eq!(wc.title, "Warchief");
        assert_eq!((wc.max_hp, wc.damage, wc.block_chance), (Some(300.0), Some(30.0), Some(0.15)));
        assert_eq!(wc.cleave, Some((34.0, 0.55)));
        assert_eq!(wc.aura.unwrap().damage_mult, 1.10);

        let c = commander_profile(Species::Goblins);
        assert_eq!(c.title, "Chieftain");
        assert_eq!((c.max_hp, c.damage, c.speed, c.dodge_chance), (Some(200.0), Some(14.0), Some(42.0), Some(0.32)));
        let a = c.aura.unwrap();
        assert_eq!((a.dodge_add, a.cooldown_mult), (0.045, 0.98));
    }

    #[test]
    fn unit_moves_at_its_speed() {
        let mut b = Battle::new();
        b.add(UnitKind::Infantry, 0, 0.0, 0.0);
        b.units[0].move_point = Some((100.0, 0.0));
        b.sim_tick(0.1); // 6 battle-seconds
        // Infantry: 34 px/s ÷ 8 = 4.25 cells/battle-second → 25.5 cells.
        let u = &b.units[0];
        assert!((u.x - 25.5).abs() < 1e-9, "x = {}", u.x);
        assert_eq!(u.y, 0.0);
        assert_eq!(u.facing, 0.0);
    }

    #[test]
    fn unit_arrives_and_stops() {
        let mut b = Battle::new();
        b.add(UnitKind::Infantry, 0, 0.0, 0.0);
        b.units[0].move_point = Some((10.0, 0.0));
        for _ in 0..200 {
            b.sim_tick(0.1);
        }
        let u = &b.units[0];
        assert!(
            (u.x - 10.0).abs() < ARRIVE_EPS + 1e-9,
            "arrived at x = {}",
            u.x
        );
        assert_eq!(u.y, 0.0);
    }

    #[test]
    fn separation_pushes_follower_off_line() {
        // A follower 2 cells behind the leader is inside personal space
        // (radii 5+5 px = 1.25 cells + 1.125 = 2.375), so it must be
        // deflected sideways rather than walking into the leader.
        let mut b = Battle::new();
        b.add(UnitKind::Infantry, 0, 0.0, 0.0); // leader
        b.add(UnitKind::Infantry, 0, -2.0, 0.0); // follower
        b.units[0].move_point = Some((50.0, 0.0));
        b.units[1].move_point = Some((50.0, 0.0));
        b.sim_tick(0.1);
        let follower = &b.units[1];
        assert!(
            follower.y.abs() > 1.0,
            "follower should be deflected sideways, y = {}",
            follower.y
        );
        // And it still makes forward progress.
        assert!(follower.x > -2.0);
    }

    #[test]
    fn deflection_is_capped() {
        // A unit boxed in from the front may not be turned more than 60°
        // off its objective: the steered heading must keep a positive
        // component along the seek.
        let mut b = Battle::new();
        b.add(UnitKind::Infantry, 0, 0.0, 0.0);
        // A wall of allies directly ahead, inside the avoid cone.
        for i in 0..5 {
            b.add(UnitKind::Infantry, 0, 2.0, -4.0 + i as f64 * 2.0);
        }
        b.units[0].move_point = Some((50.0, 0.0));
        let grid = MoveGrid::build(&b.units);
        let (vx, vy) = steer(&b.units[0], 1.0, 0.0, &grid, &b.units);
        let dot = vx * 1.0 + vy * 0.0;
        assert!(
            dot >= MAX_DEFLECT_COS - 1e-9,
            "deflection exceeded the cap: dot = {}",
            dot
        );
        // Still a unit vector.
        assert!((vx * vx + vy * vy - 1.0).abs() < 1e-9);
    }

    #[test]
    fn no_cohesion_ignores_steering() {
        let mut b = Battle::new();
        b.add(UnitKind::Berserker, 0, 0.0, 0.0);
        b.add(UnitKind::Infantry, 0, 2.0, 0.0); // an ally in the way
        b.units[0].move_point = Some((50.0, 0.0));
        let grid = MoveGrid::build(&b.units);
        let (vx, vy) = steer(&b.units[0], 1.0, 0.0, &grid, &b.units);
        assert_eq!((vx, vy), (1.0, 0.0), "berserker keeps no line");
    }

    #[test]
    fn commander_profile_applies_to_unit() {
        let mut b = Battle::new();
        let id = b.add_commander(Species::Dwarves, 0, 0.0, 0.0);
        let u = &b.units[id as usize];
        assert_eq!(u.max_hp(), 470.0, "Thane's 470 hp override");
        assert_eq!(u.hp, 470.0);
        // Dwarven Thane speed override: 28 px/s → 3.5 cells/battle-second.
        assert!((u.speed_cells() - 3.5).abs() < 1e-9);

        let id = b.add_commander(Species::Humans, 1, 0.0, 0.0);
        let u = &b.units[id as usize];
        assert_eq!(u.max_hp(), 250.0, "Marshal's 250 hp override");
    }

    #[test]
    fn mustering_levy_formula() {
        // 1000 adults → levy 80. 50 weapons → 50 armed, 30 militia.
        let m = muster_strength(1000.0, 50.0, 0.0, 0.0, 0.0, 1.0);
        assert!((m.levy - 80.0).abs() < 1e-9);
        assert!((m.armed - 50.0).abs() < 1e-9);
        assert!((m.militia - 30.0).abs() < 1e-9);
        // strength = 50 + 30×0.30 = 59.
        assert!((m.strength - 59.0).abs() < 1e-9);

        // Full shields and horses: 59 × 1.25 × 1.50 = 110.625.
        let m = muster_strength(1000.0, 50.0, 50.0, 50.0, 0.0, 1.0);
        assert!((m.strength - 110.625).abs() < 1e-9);

        // Species +10% mil and 0.9 loyalty: 110.625 × 1.10 × 0.9 = 109.51875.
        let m = muster_strength(1000.0, 50.0, 50.0, 50.0, 10.0, 0.9);
        assert!((m.strength - 109.51875).abs() < 1e-9);

        // Floor: a tiny realm still fields 10.
        let m = muster_strength(10.0, 0.0, 0.0, 0.0, 0.0, 1.0);
        assert_eq!(m.strength, MILITARY_FLOOR);

        // Ceiling: a huge realm caps at 1200.
        let m = muster_strength(100_000.0, 100_000.0, 100_000.0, 100_000.0, 0.0, 1.0);
        assert_eq!(m.strength, MILITARY_CEILING);
    }

    #[test]
    fn army_composition_is_deterministic() {
        let v = army_composition(40.0);
        assert_eq!(v.len(), 40);
        assert_eq!(v.iter().filter(|k| **k == UnitKind::Archer).count(), 10);
        assert_eq!(v.iter().filter(|k| **k == UnitKind::Infantry).count(), 30);
        assert_eq!(army_composition(40.0), v);
    }

    #[test]
    fn muster_army_deploys_within_the_field() {
        let map = crate::worldgen::generate(256, 160, 2024);
        let mut s = crate::settlement::Settlement::spawn(&map, 2024);
        s.resources.insert("Weapons".into(), 50.0);
        s.resources.insert("Shields".into(), 50.0);
        let m = muster_strength(s.adults, 50.0, 50.0, 0.0, 0.0, 1.0);
        let b = muster_army(&s, 0, Species::Humans);
        // The levy's strength in soldiers, plus the one commander on top.
        assert_eq!(b.units.len(), m.strength as usize + 1);
        for u in &b.units {
            assert!((0.0..=BATTLE_FIELD_CELLS).contains(&u.x), "x = {}", u.x);
            assert!((0.0..=BATTLE_FIELD_CELLS).contains(&u.y), "y = {}", u.y);
            assert_eq!(u.faction, 0);
        }
        // Faction 1 enters from the east edge.
        let b1 = muster_army(&s, 1, Species::Dwarves);
        assert!(b1.units.iter().all(|u| u.x > BATTLE_FIELD_CELLS / 2.0));
        assert_eq!(b1.units.last().unwrap().kind, UnitKind::Commander);
    }

    /// Two mixed armies of 60 walk at each other across the field for two
    /// days. Same seed, same result — the fingerprint.
    #[test]
    fn deterministic_fingerprint() {
        let fp = || {
            let mut b = Battle::new();
            let kinds = [UnitKind::Infantry, UnitKind::Archer, UnitKind::Cavalry];
            for i in 0..60 {
                let kind = kinds[i % 3];
                b.add(kind, 0, 10.0 + (i % 6) as f64, 40.0 + (i / 6) as f64 * 2.0);
                b.add(kind, 1, 90.0 - (i % 6) as f64, 40.0 + (i / 6) as f64 * 2.0);
            }
            b.add_commander(Species::Humans, 0, 10.0, 50.0);
            b.add_commander(Species::Dwarves, 1, 90.0, 50.0);
            // Everyone walks for the middle of the field.
            for u in b.units.iter_mut() {
                u.move_point = Some((50.0, 50.0));
            }
            let mut t = 0.0;
            while t < 2.0 {
                b.sim_tick(0.1);
                t += 0.1;
            }
            let (mut sx, mut sy, mut sc) = (0.0f64, 0.0f64, 0.0f64);
            for u in &b.units {
                sx += u.x;
                sy += u.y;
                sc += u.facing;
            }
            (b.seconds, sx, sy, sc)
        };
        let a = fp();
        let b = fp();
        assert_eq!(a, b, "same setup must produce identical state");
        // Sanity: the armies actually moved toward the centre.
        assert!(a.1 > 0.0 && a.1 < 12200.0);
    }
}
