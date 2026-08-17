//! Natural world generation — a port of `app/world/worldgen.py`'s terrain
//! pipeline (plates → elevation → sea level → hydrology → climate → biome),
//! plus the one thing the Python version never had: **erosion**. Thermal
//! erosion relaxes the plate-generated slopes into talus, and a fluvial
//! (stream-power) carve deepens river valleys along the D8 flow network — so
//! mountains read as weathered and rivers sit in valleys, not on the surface.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::grid::{BoolGrid, Grid};
use crate::noise::{fbm_grid, periodic_octaves, vhash};
use crate::plates::{generate_plates, height_contribution};
use crate::rng::Rng;

// --- biome ids ---------------------------------------------------------------
// A wider palette than the original game: more climate bands, polar ice, a
// snow line, and separate wetland kinds, so a map reads as a real gradient of
// biomes rather than a handful of blocks.
pub const BIOME_OCEAN: u8 = 0;
pub const BIOME_ICE: u8 = 1;
pub const BIOME_TUNDRA: u8 = 2;
pub const BIOME_ALPINE: u8 = 3;
pub const BIOME_TAIGA: u8 = 4;
pub const BIOME_BOREAL: u8 = 5;
pub const BIOME_STEPPE: u8 = 6;
pub const BIOME_GRASSLAND: u8 = 7;
pub const BIOME_PLAINS: u8 = 8;
pub const BIOME_TEMPERATE_FOREST: u8 = 9;
pub const BIOME_TEMPERATE_RAINFOREST: u8 = 10;
pub const BIOME_SHRUBLAND: u8 = 11;
pub const BIOME_DESERT: u8 = 12;
pub const BIOME_SAVANNAH: u8 = 13;
pub const BIOME_MONSOON: u8 = 14;
pub const BIOME_JUNGLE: u8 = 15;
pub const BIOME_MOUNTAIN: u8 = 16;
pub const BIOME_SNOW_PEAK: u8 = 17;
pub const BIOME_HIGHLAND: u8 = 18;
pub const BIOME_COASTAL: u8 = 19;
pub const BIOME_SWAMP: u8 = 20;
pub const BIOME_MARSH: u8 = 21;
pub const BIOME_MANGROVE: u8 = 22;

pub fn biome_name(b: u8) -> &'static str {
    match b {
        BIOME_OCEAN => "ocean",
        BIOME_ICE => "ice",
        BIOME_TUNDRA => "tundra",
        BIOME_ALPINE => "alpine",
        BIOME_TAIGA => "taiga",
        BIOME_BOREAL => "boreal forest",
        BIOME_STEPPE => "steppe",
        BIOME_GRASSLAND => "grassland",
        BIOME_PLAINS => "plains",
        BIOME_TEMPERATE_FOREST => "temperate forest",
        BIOME_TEMPERATE_RAINFOREST => "temperate rainforest",
        BIOME_SHRUBLAND => "shrubland",
        BIOME_DESERT => "desert",
        BIOME_SAVANNAH => "savannah",
        BIOME_MONSOON => "monsoon forest",
        BIOME_JUNGLE => "jungle",
        BIOME_MOUNTAIN => "mountain",
        BIOME_SNOW_PEAK => "snow peak",
        BIOME_HIGHLAND => "highland",
        BIOME_COASTAL => "coastal",
        BIOME_SWAMP => "swamp",
        BIOME_MARSH => "marsh",
        BIOME_MANGROVE => "mangrove",
        _ => "unknown",
    }
}

// --- biome / climate thresholds ---------------------------------------------
const ICE_TEMP: f64 = 0.12; // polar ice caps below this temperature
const COLD_TEMP: f64 = 0.40; // cold / temperate boundary
const WARM_TEMP: f64 = 0.60; // temperate / warm boundary
const SNOW_RELIEF: f64 = 0.78; // relief above which peaks are snow-capped
const MOUNTAIN_RELIEF: f64 = 0.60;
const HIGHLAND_RELIEF: f64 = 0.40;
const COASTAL_REACH: f64 = 3.0;
const SWAMP_MOISTURE: f64 = 0.68;
const SWAMP_RELIEF_MAX: f64 = 0.18;
const SWAMP_WATER_REACH: f64 = 3.0;
const TEMP_BANDS: [f64; 3] = [0.40, 0.60, 0.80]; // cold | temperate | warm | hot
const MOISTURE_BANDS: [f64; 4] = [0.20, 0.40, 0.60, 0.80]; // arid | dry | moderate | moist | wet
// rows: cold / temperate / warm / hot; cols: arid / dry / moderate / moist / wet
const BIOME_MATRIX: [[u8; 5]; 4] = [
    [BIOME_TUNDRA, BIOME_TUNDRA, BIOME_TAIGA, BIOME_TAIGA, BIOME_BOREAL],
    [BIOME_STEPPE, BIOME_GRASSLAND, BIOME_PLAINS, BIOME_TEMPERATE_FOREST, BIOME_TEMPERATE_RAINFOREST],
    [BIOME_DESERT, BIOME_SHRUBLAND, BIOME_SAVANNAH, BIOME_MONSOON, BIOME_JUNGLE],
    [BIOME_DESERT, BIOME_SAVANNAH, BIOME_MONSOON, BIOME_JUNGLE, BIOME_JUNGLE],
];

// --- moisture drivers (worldgen.py phase F) ---------------------------------
const MOISTURE_BASE: f64 = 0.48;
const SUBTROPIC_DRY: f64 = 0.22;
const SUBTROPIC_CENTER: f64 = 0.28;
const SUBTROPIC_WIDTH: f64 = 0.15;
const EQUATOR_BOOST: f64 = 0.16;
const EQUATOR_BOOST_WIDTH: f64 = 0.25;
const POLE_DROP: f64 = 0.16;
const POLE_DROP_START: f64 = 0.6;
const COASTAL_MOISTURE: f64 = 0.26;
const COASTAL_FALLOFF: f64 = 16.0;
const RIPARIAN_MOISTURE: f64 = 0.18;
const RIPARIAN_FALLOFF: f64 = 3.0;
const OROGRAPHIC_RAIN_RELIEF: f64 = 0.30;
const OROGRAPHIC_BARRIER_RELIEF: f64 = 0.45;
const OROGRAPHIC_RAIN_STRENGTH: f64 = 0.20;
const OROGRAPHIC_SHADOW_STRENGTH: f64 = 0.30;
const OROGRAPHIC_SHADOW_LENGTH: f64 = 28.0;
const MOISTURE_NOISE_GAIN: f64 = 0.16;
const MOISTURE_NOISE_OCTAVES: [(f64, f64); 3] = [(0.05, 1.0), (0.11, 0.5), (0.24, 0.25)];

const ELEVATION_LAPSE: f64 = 0.45;
const DETAIL_AMPLITUDE: f64 = 0.30;
const LAKE_DEPTH: f64 = 0.011;
const MIN_ISLAND_SHARE: f64 = 0.010;
const RIVER_TIE_EPS: f64 = 0.004;

const NEIGH8: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];
const NEIGH4: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

pub struct WorldMap {
    pub w: i32,
    pub h: i32,
    pub height: Grid,
    pub sea_level: f64,
    pub land: BoolGrid,
    pub biome: Vec<u8>,
    pub river: BoolGrid,
    pub lake: BoolGrid,
    pub ocean_depth: Grid, // ocean cells: depth below sea level (continuous); land: 0
    pub moisture: Grid,
    pub temperature: Grid,
    pub n_land: usize,
    pub n_continents: usize,
    pub n_river_cells: usize,
}

#[derive(PartialEq)]
struct F64Ord(f64);
impl Eq for F64Ord {}
impl PartialOrd for F64Ord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for F64Ord {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

fn pick_n_plates(w: i32, h: i32, rng: &mut Rng) -> i32 {
    // More plates than the Python's 11·√area (which targets a 1100×660 map):
    // a small map still needs enough continental plates — separated by oceanic
    // ones — to read as several continents, not one supercontinent.
    let area_ratio = (w * h) as f64 / (1100.0 * 660.0);
    let base = 16.0 * area_ratio.sqrt();
    (base * rng.range_f64(0.85, 1.15)).round().max(10.0) as i32
}

fn percentile(v: &[f64], frac: f64) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let idx = ((s.len() as f64 * frac) as usize).min(s.len() - 1);
    s[idx]
}

/// 4-neighbour BFS distance from a set of source cells; unreachable = 1e9.
fn bfs_distance(w: i32, h: i32, sources: &[bool]) -> Grid {
    let mut dist = Grid::new(w, h, 1e9);
    let mut q = std::collections::VecDeque::new();
    for y in 0..h {
        for x in 0..w {
            if sources[(y * w + x) as usize] {
                dist.set(x, y, 0.0);
                q.push_back((x, y));
            }
        }
    }
    while let Some((x, y)) = q.pop_front() {
        let d = dist.get(x, y);
        for &(dx, dy) in &NEIGH4 {
            let nx = (x + dx).rem_euclid(w);
            let ny = y + dy;
            if ny < 0 || ny >= h {
                continue;
            }
            if dist.get(nx, ny) > d + 1.0 {
                dist.set(nx, ny, d + 1.0);
                q.push_back((nx, ny));
            }
        }
    }
    dist
}

/// Talus relaxation: move material from too-steep cells onto their steepest
/// downhill neighbour, on land only. Deterministic.
fn thermal_erosion(height: &mut Grid, land: &BoolGrid, iterations: i32) {
    let (w, h) = (height.w, height.h);
    let talus = 0.004; // max stable drop per cell before material slides
    for _ in 0..iterations {
        let mut delta = Grid::new(w, h, 0.0);
        for y in 0..h {
            for x in 0..w {
                if !land.get(x, y) {
                    continue;
                }
                let cur = height.get(x, y);
                let mut max_drop = 0.0f64;
                let mut tx = x;
                let mut ty = y;
                for &(dx, dy) in &NEIGH8 {
                    let nx = (x + dx).rem_euclid(w);
                    let ny = y + dy;
                    if ny < 0 || ny >= h || !land.get(nx, ny) {
                        continue;
                    }
                    let drop = cur - height.get(nx, ny);
                    if drop > max_drop {
                        max_drop = drop;
                        tx = nx;
                        ty = ny;
                    }
                }
                if max_drop > talus {
                    let amt = (max_drop - talus) * 0.5;
                    delta.set(x, y, delta.get(x, y) - amt);
                    delta.set(tx, ty, delta.get(tx, ty) + amt);
                }
            }
        }
        for i in 0..delta.v.len() {
            if delta.v[i] != 0.0 {
                height.v[i] += delta.v[i];
            }
        }
    }
}

/// Ease the coastline into a gradual coastal plain (land) and shallow shelf
/// (ocean), so plate boundaries and mountain ranges don't cliff straight into
/// the sea. Works off the provisional coast, then the caller re-normalises and
/// re-thresholds.
fn coastal_ramp(height: &mut Grid, land: &BoolGrid, sea_level: f64, nseed: i64) {
    let (w, h) = (height.w, height.h);
    let n = (w * h) as usize;
    let mut ocean = vec![false; n];
    let mut land_src = vec![false; n];
    for i in 0..n {
        ocean[i] = !land.v[i];
        land_src[i] = land.v[i];
    }
    // Blur the integer BFS distance into a smooth gradient, so the smoothstep
    // below doesn't stamp concentric contour bands into the coastal plain/shelf.
    let coast = bfs_distance(w, h, &ocean).blur(2, 3); // land -> steps to open ocean
    let shelf = bfs_distance(w, h, &land_src).blur(2, 3); // ocean -> steps to land

    // Fractal detail for coastline roughening: a bay/peninsula-scale wiggle
    // that breaks up the smooth plate arcs into an irregular shoreline.
    let rough_oct = periodic_octaves(w, &[(0.07, 1.0), (0.15, 0.55), (0.30, 0.30)]);
    let rough = fbm_grid(w, h, nseed + 777, &rough_oct, None, None);
    let rough_mean = rough.mean();

    const PLAIN: f64 = 16.0; // cells of coastal plain
    const SHELF: f64 = 14.0; // cells of shallow shelf
    const EASE: f64 = 0.6; // how far toward the target we pull (keeps relief)
    const ROUGH_RADIUS: f64 = 12.0; // cells over which the shore roughens
    const ROUGH_AMP: f64 = 0.07; // normalized-height wiggle at the shoreline

    for i in 0..n {
        let d = if land.v[i] { coast.v[i] } else { shelf.v[i] };
        let rw = if d <= ROUGH_RADIUS {
            let t = 1.0 - d / ROUGH_RADIUS;
            t * t * (3.0 - 2.0 * t)
        } else {
            0.0
        };
        height.v[i] += ROUGH_AMP * (rough.v[i] - rough_mean) * rw;

        if land.v[i] {
            if d <= PLAIN {
                let t = 1.0 - d / PLAIN;
                let t = t * t * (3.0 - 2.0 * t); // 1 at the coast, 0 inland
                let target = sea_level * 1.04; // a low plain just above sea
                height.v[i] += (target - height.v[i]) * t * EASE;
            }
        } else if d <= SHELF {
            let t = 1.0 - d / SHELF;
            let t = t * t * (3.0 - 2.0 * t);
            let target = sea_level * 0.90; // a shallow shelf just below sea
            height.v[i] += (target - height.v[i]) * t * EASE;
        }
    }
}

/// Priority-flood fill (Barnes): raise pits so every land cell drains to the
/// sea. Returns the filled DEM and the set of land cells that became lakes.
fn priority_flood(height: &Grid, land: &BoolGrid) -> (Grid, BoolGrid) {
    let (w, h) = (height.w, height.h);
    let mut filled = height.clone();
    let mut done = BoolGrid::new(w, h, false);
    let mut pq: BinaryHeap<std::cmp::Reverse<(F64Ord, i32, i32)>> = BinaryHeap::new();
    for y in 0..h {
        for x in 0..w {
            if !land.get(x, y) {
                done.set(x, y, true);
                pq.push(std::cmp::Reverse((F64Ord(height.get(x, y)), x, y)));
            }
        }
    }
    let eps = 1e-5;
    while let Some(std::cmp::Reverse((F64Ord(e), x, y))) = pq.pop() {
        for &(dx, dy) in &NEIGH8 {
            let nx = (x + dx).rem_euclid(w);
            let ny = y + dy;
            if ny < 0 || ny >= h || done.get(nx, ny) {
                continue;
            }
            done.set(nx, ny, true);
            let ne = if height.get(nx, ny) > e + eps {
                height.get(nx, ny)
            } else {
                e + eps
            };
            filled.set(nx, ny, ne);
            pq.push(std::cmp::Reverse((F64Ord(ne), nx, ny)));
        }
    }

    let mut lake = BoolGrid::new(w, h, false);
    for y in 0..h {
        for x in 0..w {
            if land.get(x, y) && filled.get(x, y) - height.get(x, y) > LAKE_DEPTH {
                lake.set(x, y, true);
            }
        }
    }
    (filled, lake)
}

/// D8 flow direction + accumulation on the filled DEM. `down` is a cell index
/// or -1 (no flow / flows to ocean). Slope-weighted steepest descent with a
/// deterministic near-tie break, then a cycle-break pass.
fn d8_flow(
    w: i32,
    h: i32,
    filled: &Grid,
    height: &Grid,
    land: &BoolGrid,
    rseed: i64,
) -> (Vec<i32>, Vec<f64>, Vec<f64>, Vec<Vec<(f64, i32, i32)>>) {
    let n = (w * h) as usize;
    let sqrt2 = 2.0f64.sqrt();
    let mut down = vec![-1i32; n];
    let mut flow_best = vec![0.0f64; n];
    let mut flow_near: Vec<Vec<(f64, i32, i32)>> = vec![Vec::new(); n];

    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if !land.get(x, y) {
                continue;
            }
            let c = filled.get(x, y);
            let mut best_score = 0.0f64;
            let mut near: Vec<(f64, i32, i32)> = Vec::new();
            for &(dx, dy) in &NEIGH8 {
                let nx = (x + dx).rem_euclid(w);
                let ny = y + dy;
                if ny < 0 || ny >= h {
                    continue;
                }
                let fe = if land.get(nx, ny) {
                    filled.get(nx, ny)
                } else {
                    height.get(nx, ny)
                };
                let dist = if dx == 0 || dy == 0 { 1.0 } else { sqrt2 };
                let score = (c - fe) / dist;
                if score >= 0.0 {
                    near.push((score, nx, ny));
                    if score > best_score {
                        best_score = score;
                    }
                }
            }
            if best_score > 0.0 {
                flow_best[i] = best_score;
                near.retain(|&(s, _, _)| s >= best_score - RIVER_TIE_EPS);
                // among the equivalent drops, the largest per-neighbour noise wins
                let (_s, bx, by) = near
                    .iter()
                    .max_by(|a, b| {
                        vhash(a.1 as i64, a.2 as i64, rseed, Some(w as i64))
                            .total_cmp(&vhash(b.1 as i64, b.2 as i64, rseed, Some(w as i64)))
                    })
                    .unwrap();
                down[i] = (by * w + bx) as i32;
                flow_near[i] = near;
            }
            // else: flat — no flow, river ends here
        }
    }

    // Break flow cycles: redirect the weakest member of each cycle.
    for _pass in 0..8 {
        let mut colour = vec![0u8; n];
        let mut found = false;
        for start_y in 0..h {
            for start_x in 0..w {
                let start = (start_y * w + start_x) as usize;
                if !land.get(start_x, start_y) || colour[start] != 0 {
                    continue;
                }
                let mut path: Vec<usize> = Vec::new();
                let mut cur: i32 = start as i32;
                while cur >= 0 && colour[cur as usize] == 0 {
                    colour[cur as usize] = 1;
                    path.push(cur as usize);
                    cur = down[cur as usize];
                }
                if cur >= 0 && colour[cur as usize] == 1 {
                    found = true;
                    let cycle_start = path.iter().position(|&p| p == cur as usize).unwrap();
                    let cyc = &path[cycle_start..];
                    let cyc_set: std::collections::HashSet<usize> = cyc.iter().cloned().collect();
                    let path_set: std::collections::HashSet<usize> = path.iter().cloned().collect();
                    let mut cyc_sorted: Vec<usize> = cyc.to_vec();
                    cyc_sorted.sort_by(|a, b| flow_best[*a].total_cmp(&flow_best[*b]));
                    for &cell in &cyc_sorted {
                        let mut redirected = false;
                        for &(_s, nx, ny) in &flow_near[cell] {
                            let ni = (ny * w + nx) as usize;
                            if !cyc_set.contains(&ni) && !path_set.contains(&ni) {
                                down[cell] = ni as i32;
                                redirected = true;
                                break;
                            }
                        }
                        if redirected {
                            break;
                        }
                    }
                }
                for &p in &path {
                    colour[p] = 2;
                }
            }
        }
        if !found {
            break;
        }
    }

    // Flow accumulation: high cells first, push water downstream.
    let mut order: Vec<(usize, f64)> = (0..n)
        .filter(|&i| land.v[i])
        .map(|i| (i, filled.v[i]))
        .collect();
    order.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut acc = vec![1.0f64; n];
    for (i, _) in order {
        let d = down[i];
        if d >= 0 && land.v[d as usize] {
            acc[d as usize] += acc[i];
        }
    }
    (down, acc, flow_best, flow_near)
}

/// Stream-power carve: erode ∝ √flow × slope along the D8 network, deposit
/// downstream. Deterministic; carves the river valleys into the terrain.
fn fluvial_carve(
    height: &mut Grid,
    land: &BoolGrid,
    down: &[i32],
    acc: &[f64],
    sea_level: f64,
    iterations: i32,
) {
    let (w, h) = (height.w, height.h);
    let k = 0.0005;
    for _ in 0..iterations {
        let mut delta = Grid::new(w, h, 0.0);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                if !land.get(x, y) || height.get(x, y) <= sea_level + 0.02 {
                    continue;
                }
                let d = down[i];
                if d < 0 {
                    continue;
                }
                let (dx, dy) = ((d % w) as i32, (d / w) as i32);
                let slope = (height.get(x, y) - height.get(dx, dy)).max(0.0);
                let erode = k * acc[i].sqrt() * slope;
                if erode > 0.0 {
                    delta.set(x, y, delta.get(x, y) - erode);
                    delta.set(dx, dy, delta.get(dx, dy) + erode);
                }
            }
        }
        for i in 0..delta.v.len() {
            if delta.v[i] != 0.0 {
                height.v[i] += delta.v[i];
            }
        }
    }
}

fn latitude_moisture(h: i32) -> Vec<f64> {
    (0..h)
        .map(|y| {
            let t = (y as f64 / h as f64 - 0.5).abs() * 2.0;
            let dry = SUBTROPIC_DRY * (-((t - SUBTROPIC_CENTER) / SUBTROPIC_WIDTH).powi(2)).exp();
            let wet = EQUATOR_BOOST * (-(t / EQUATOR_BOOST_WIDTH).powi(2)).exp();
            let pole = POLE_DROP
                * (((t - POLE_DROP_START) / (1.0 - POLE_DROP_START)).clamp(0.0, 1.0)).powi(2);
            (MOISTURE_BASE - dry + wet - pole).clamp(0.0, 1.0)
        })
        .collect()
}

/// Prevailing wind eastward component per row — a simple three-cell model:
/// easterlies (westward) at equator and poles, westerlies (eastward) between.
fn wind_u(y: i32, h: i32) -> f64 {
    let alat = (y as f64 / h as f64 - 0.5).abs();
    if alat < 0.30 || alat >= 0.55 {
        -1.0
    } else {
        1.0
    }
}

fn orography(w: i32, h: i32, height: &Grid, land: &BoolGrid, sea_level: f64) -> Grid {
    let mut wet = Grid::new(w, h, 0.0);
    let mut shadow = Grid::new(w, h, 0.0);
    let span = (1.0 - sea_level).max(1e-9);
    let inf = 1e9f64;
    for y in 0..h {
        let direction = if wind_u(y, h) >= 0.0 { 1 } else { -1 };
        let xs: Vec<i32> = if direction > 0 {
            (0..w).collect()
        } else {
            (0..w).rev().collect()
        };
        // WET pass: rain on rising, high-enough flanks.
        let mut prev = height.get(xs[0], y);
        for &x in &xs {
            let e = height.get(x, y);
            if land.get(x, y) {
                let relief = (e - sea_level) / span;
                let slope = e - prev;
                if slope > 0.0 && relief > OROGRAPHIC_RAIN_RELIEF {
                    wet.set(x, y, OROGRAPHIC_RAIN_STRENGTH * (slope * 20.0).min(1.0));
                }
            }
            prev = e;
        }
        // DRY pass: distance downwind from the last high ridge.
        let mut dist = inf;
        for &x in &xs {
            let e = height.get(x, y);
            if !land.get(x, y) {
                dist = inf; // ocean re-humidifies
            } else {
                let relief = (e - sea_level) / span;
                if relief > OROGRAPHIC_BARRIER_RELIEF {
                    dist = 0.0;
                } else if dist < inf {
                    dist += 1.0;
                }
            }
            if dist < inf {
                shadow.set(x, y, OROGRAPHIC_SHADOW_STRENGTH * (-dist / OROGRAPHIC_SHADOW_LENGTH).exp());
            }
        }
    }
    let mut out = Grid::new(w, h, 0.0);
    for i in 0..out.v.len() {
        out.v[i] = wet.v[i] - shadow.v[i];
    }
    out
}

fn compute_moisture(
    w: i32,
    h: i32,
    land: &BoolGrid,
    height: &Grid,
    sea_level: f64,
    river_lake: &BoolGrid,
    nseed: i64,
) -> Grid {
    // coastal distance: BFS from ocean cells.
    let mut ocean = vec![false; (w * h) as usize];
    for i in 0..ocean.len() {
        ocean[i] = !land.v[i];
    }
    let coast = bfs_distance(w, h, &ocean);
    let water_src = river_lake.clone();
    let water = bfs_distance(w, h, &water_src.v);

    let lat = latitude_moisture(h);
    let oro = orography(w, h, height, land, sea_level);
    let mseed = (nseed ^ 0x9E37_79B9) as i64;
    let octaves = periodic_octaves(w, &MOISTURE_NOISE_OCTAVES);
    let tex = fbm_grid(w, h, mseed, &octaves, None, None);
    let tex_max = tex.max().max(1e-9);

    let mut out = Grid::new(w, h, 0.0);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if !land.v[i] {
                continue;
            }
            let coastal = COASTAL_MOISTURE * (-coast.v[i] / COASTAL_FALLOFF).exp();
            let riparian = RIPARIAN_MOISTURE * (-water.v[i] / RIPARIAN_FALLOFF).exp();
            let field = lat[y as usize] + coastal + riparian + oro.v[i]
                + MOISTURE_NOISE_GAIN * (tex.v[i] / tex_max - 0.5);
            out.v[i] = field.clamp(0.0, 1.0);
        }
    }
    out
}

fn classify_biome(
    relief: f64,
    moisture: f64,
    coast_dist: f64,
    water_dist: f64,
    temperature: f64,
) -> u8 {
    // Polar ice caps, then a snow line, beat the climate matrix outright.
    if temperature < ICE_TEMP {
        return BIOME_ICE;
    }
    if relief > SNOW_RELIEF {
        return BIOME_SNOW_PEAK;
    }
    if relief > MOUNTAIN_RELIEF {
        return BIOME_MOUNTAIN;
    }
    if relief > HIGHLAND_RELIEF {
        // Above the treeline: alpine tundra in the cold, foothills in the warm.
        return if temperature < COLD_TEMP {
            BIOME_ALPINE
        } else {
            BIOME_HIGHLAND
        };
    }
    if coast_dist <= COASTAL_REACH {
        // Tropical wet coastlines are mangrove, not just sand.
        return if temperature >= WARM_TEMP && moisture > SWAMP_MOISTURE {
            BIOME_MANGROVE
        } else {
            BIOME_COASTAL
        };
    }
    // Wet low ground: frozen bog in the cold, swamp in the warm.
    if moisture > SWAMP_MOISTURE && relief < SWAMP_RELIEF_MAX && water_dist <= SWAMP_WATER_REACH {
        return if temperature < COLD_TEMP {
            BIOME_MARSH
        } else {
            BIOME_SWAMP
        };
    }
    let tband = TEMP_BANDS
        .iter()
        .position(|&e| temperature < e)
        .unwrap_or(TEMP_BANDS.len());
    let mband = MOISTURE_BANDS
        .iter()
        .position(|&e| moisture < e)
        .unwrap_or(MOISTURE_BANDS.len());
    BIOME_MATRIX[tband][mband]
}

/// Flood-fill land components; sink every one smaller than `share` of total
/// land except the `keep_largest` biggest.
fn cull_small_islands(height: &mut Grid, land: &mut BoolGrid, sea_level: f64, keep_largest: usize) {
    let (w, h) = (height.w, height.h);
    let n = (w * h) as usize;
    let mut seen = vec![false; n];
    let mut comps: Vec<Vec<usize>> = Vec::new();
    for y0 in 0..h {
        for x0 in 0..w {
            let i0 = (y0 * w + x0) as usize;
            if !land.v[i0] || seen[i0] {
                continue;
            }
            let mut comp = Vec::new();
            let mut stack = vec![(x0, y0)];
            seen[i0] = true;
            while let Some((x, y)) = stack.pop() {
                comp.push((y * w + x) as usize);
                for &(dx, dy) in &NEIGH8 {
                    let nx = (x + dx).rem_euclid(w);
                    let ny = y + dy;
                    if ny < 0 || ny >= h {
                        continue;
                    }
                    let ni = (ny * w + nx) as usize;
                    if land.v[ni] && !seen[ni] {
                        seen[ni] = true;
                        stack.push((nx, ny));
                    }
                }
            }
            comps.push(comp);
        }
    }
    let total_land: usize = comps.iter().map(|c| c.len()).sum();
    if total_land == 0 {
        return;
    }
    let min_keep = (total_land as f64 * MIN_ISLAND_SHARE).round() as usize;
    comps.sort_by(|a, b| b.len().cmp(&a.len()));
    for (rank, comp) in comps.iter().enumerate() {
        if rank < keep_largest || comp.len() >= min_keep {
            continue;
        }
        for &i in comp {
            height.v[i] = sea_level - 0.001;
            land.v[i] = false;
        }
    }
}

/// Generate a natural world, retrying (with a stepped seed) until the land
/// reads as several continents rather than one supercontinent.
pub fn generate(w: i32, h: i32, seed: u64) -> WorldMap {
    let mut attempt_seed = seed;
    for attempt in 0..8 {
        let map = generate_once(w, h, attempt_seed);
        if map.n_continents >= 3 || attempt == 7 {
            return map;
        }
        attempt_seed = attempt_seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    }
    unreachable!()
}

fn generate_once(w: i32, h: i32, seed: u64) -> WorldMap {
    let mut rng = Rng::new(seed);
    let nseed = rng.next_u64() as i64;
    let n_plates = pick_n_plates(w, h, &mut rng);

    // 1. plate-driven height field.
    let plate_seed = rng.next_u64();
    let pl = generate_plates(w, h, plate_seed, n_plates);
    let mut v = height_contribution(&pl);

    // 2. domain-warped detail noise.
    let height_oct = periodic_octaves(w, &[(0.028, 1.0), (0.060, 0.5), (0.130, 0.25), (0.260, 0.07)]);
    let warp_oct = periodic_octaves(w, &[(0.018, 1.0), (0.040, 0.22)]);
    let warp_amp = w as f64 * 0.05;
    let mut wx = fbm_grid(w, h, nseed + 101, &warp_oct, None, None);
    let mut wy = fbm_grid(w, h, nseed + 202, &warp_oct, None, None);
    let (wxm, wym) = (wx.mean(), wy.mean());
    for i in 0..wx.v.len() {
        wx.v[i] = (wx.v[i] - wxm) * 2.0 * warp_amp;
        wy.v[i] = (wy.v[i] - wym) * 2.0 * warp_amp;
    }
    let detail = fbm_grid(w, h, nseed, &height_oct, Some(&wx), Some(&wy));
    let detail_mean = detail.mean();
    for i in 0..v.v.len() {
        v.v[i] += DETAIL_AMPLITUDE * (detail.v[i] - detail_mean);
    }

    // 3. meandering deep-ocean seam at the east-west wrap.
    let seam_margin = ((w as f64 * 0.03).round() as i32).max(6);
    let meander_amp = 0.55 * seam_margin as f64;
    let mut meander = vec![0.0f64; h as usize];
    let mut floor = vec![0.0f64; h as usize];
    let mut meander_raw = vec![0.0f64; h as usize];
    for y in 0..h {
        meander_raw[y as usize] = 1.0 * crate::noise::value_noise(0.0, y as f64 * 0.006, nseed + 303, None)
            + 0.45 * crate::noise::value_noise(0.0, y as f64 * 0.012, nseed + 304, None);
        floor[y as usize] = crate::noise::value_noise(0.0, y as f64 * 0.004, nseed + 404, None);
    }
    let mr_mean = meander_raw.iter().sum::<f64>() / h as f64;
    let mr_max = meander_raw.iter().map(|x| (x - mr_mean).abs()).fold(0.0f64, f64::max).max(1e-9);
    for y in 0..h {
        meander[y as usize] = (meander_raw[y as usize] - mr_mean) / mr_max * meander_amp;
        // Just below the oceanic base (-0.90), so the wrap reads as ordinary
        // ocean — no harsh dark band around the map edge — while still
        // guaranteeing no land straddles the east-west seam.
        floor[y as usize] = -(1.0 + 0.2 * floor[y as usize]);
    }
    for y in 0..h {
        let p = meander[y as usize];
        for x in 0..w {
            let mut d = (x as f64 - p).rem_euclid(w as f64);
            d = d.min(w as f64 - d);
            let fade = ((d - meander_amp) / (seam_margin as f64 - meander_amp).max(1e-9)).clamp(0.0, 1.0);
            let fade = fade * fade * (3.0 - 2.0 * fade);
            let i = (y * w + x) as usize;
            v.v[i] = v.v[i] * fade + floor[y as usize] * (1.0 - fade);
        }
    }

    // 4. normalise, sea level (land ~40%), land mask.
    let (lo, hi) = (v.min(), v.max());
    let span = (hi - lo).max(1e-9);
    for i in 0..v.v.len() {
        v.v[i] = (v.v[i] - lo) / span;
    }
    let mut sea_level = percentile(&v.v, 0.58);
    let mut land = BoolGrid::new(w, h, false);
    for y in 0..h {
        for x in 0..w {
            land.set(x, y, v.get(x, y) > sea_level);
        }
    }
    // Seam safety: sink any land on the wrap columns.
    for y in 0..h {
        for &ex in &[0, w - 1] {
            if land.get(ex, y) {
                v.set(ex, y, v.min());
                land.set(ex, y, false);
            }
        }
    }
    cull_small_islands(&mut v, &mut land, sea_level, 5);

    // 4b. gradual + weathered coastline: ease coastal land down into a plain
    // and coastal ocean up into a shelf, and add a bay/peninsula wiggle. Then
    // re-normalise + re-threshold: the shelf spreads the ocean's depth range,
    // which is what keeps the sea level (and so the land relief) balanced.
    coastal_ramp(&mut v, &land, sea_level, nseed);
    let (lo, hi) = (v.min(), v.max());
    let span = (hi - lo).max(1e-9);
    for i in 0..v.v.len() {
        v.v[i] = (v.v[i] - lo) / span;
    }
    sea_level = percentile(&v.v, 0.58);
    for y in 0..h {
        for x in 0..w {
            land.set(x, y, v.get(x, y) > sea_level);
        }
    }
    // Keep the seam invariant after the ramp (land never straddles the wrap).
    for y in 0..h {
        for &ex in &[0, w - 1] {
            if land.get(ex, y) {
                v.set(ex, y, v.min());
                land.set(ex, y, false);
            }
        }
    }
    cull_small_islands(&mut v, &mut land, sea_level, 5);

    // 5. erosion refinement (not in the Python pipeline).
    thermal_erosion(&mut v, &land, 30);

    // 6. hydrology: fill pits, D8 flow, carve valleys, then re-flow on the
    //    carved terrain so rivers follow their own valleys.
    let (filled, _lake) = priority_flood(&v, &land);
    let rseed = rng.next_u64() as i64;
    let (down, acc, _fb, _fn) = d8_flow(w, h, &filled, &v, &land, rseed);
    fluvial_carve(&mut v, &land, &down, &acc, sea_level, 20);
    let (filled2, lake2) = priority_flood(&v, &land);
    let (_down2, acc2, _fb2, _fn2) = d8_flow(w, h, &filled2, &v, &land, rseed);

    // River cells: flow accumulation above a threshold, not in a lake.
    let n_land = (0..land.v.len()).filter(|&i| land.v[i]).count();
    let thresh = (n_land as f64 / 700.0).max(35.0);
    let mut river = BoolGrid::new(w, h, false);
    let mut n_river_cells = 0;
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if land.v[i] && !lake2.v[i] && acc2[i] >= thresh {
                river.set(x, y, true);
                n_river_cells += 1;
            }
        }
    }

    // 7. climate + biome.
    let mut river_lake = BoolGrid::new(w, h, false);
    for i in 0..river_lake.v.len() {
        river_lake.v[i] = river.v[i] || lake2.v[i];
    }
    let moisture = compute_moisture(w, h, &land, &v, sea_level, &river_lake, nseed);

    let mut ocean_src = vec![false; (w * h) as usize];
    for i in 0..ocean_src.len() {
        ocean_src[i] = !land.v[i];
    }
    let coast = bfs_distance(w, h, &ocean_src);
    let water = bfs_distance(w, h, &river_lake.v);

    let mut biome = vec![BIOME_OCEAN; (w * h) as usize];
    let mut temperature = Grid::new(w, h, 0.0);
    let span = (1.0 - sea_level).max(1e-9);
    for y in 0..h {
        let latitude_temp = 1.0 - (y as f64 / h as f64 - 0.5).abs() * 2.0;
        for x in 0..w {
            let i = (y * w + x) as usize;
            if !land.v[i] {
                continue;
            }
            let relief = ((v.v[i] - sea_level) / span).clamp(0.0, 1.0);
            let temp = (latitude_temp - ELEVATION_LAPSE * relief).clamp(0.0, 1.0);
            temperature.v[i] = temp;
            biome[i] = classify_biome(relief, moisture.v[i], coast.v[i], water.v[i], temp);
        }
    }

    // Ocean depth, continuous: how far below sea level the floor sits. Using
    // the elevation field (not a BFS distance from land) avoids the concentric
    // "contour line" banding a step distance produces in the shallow water, and
    // the plate/detail noise already gives it natural bathymetry.
    let mut ocean_depth = Grid::new(w, h, 0.0);
    for i in 0..ocean_depth.v.len() {
        if !land.v[i] {
            ocean_depth.v[i] = (sea_level - v.v[i]).max(0.0);
        }
    }

    // Continent count: land components.
    let mut seen = vec![false; (w * h) as usize];
    let mut n_continents = 0;
    for y0 in 0..h {
        for x0 in 0..w {
            let i0 = (y0 * w + x0) as usize;
            if !land.v[i0] || seen[i0] {
                continue;
            }
            n_continents += 1;
            let mut stack = vec![(x0, y0)];
            seen[i0] = true;
            while let Some((x, y)) = stack.pop() {
                for &(dx, dy) in &NEIGH8 {
                    let nx = (x + dx).rem_euclid(w);
                    let ny = y + dy;
                    if ny < 0 || ny >= h {
                        continue;
                    }
                    let ni = (ny * w + nx) as usize;
                    if land.v[ni] && !seen[ni] {
                        seen[ni] = true;
                        stack.push((nx, ny));
                    }
                }
            }
        }
    }

    WorldMap {
        w,
        h,
        height: v,
        sea_level,
        land,
        biome,
        river,
        lake: lake2,
        ocean_depth,
        moisture,
        temperature,
        n_land,
        n_continents,
        n_river_cells,
    }
}
