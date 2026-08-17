//! Tectonic plates: plate territories grown over a domain-warped grid,
//! boundary classification, and the height-field contribution — a port of
//! `app/world/plates.py`. Continental plates bias toward land, oceanic toward
//! sea; collisions raise mountain ranges, rifts drop rift valleys, subduction
//! raises a coastal range on one side and a trench on the other, and hotspots
//! stamp island chains.

use crate::grid::Grid;
use crate::noise::{fbm_grid, periodic_octaves};
use crate::rng::Rng;

pub const CONTINENTAL: u8 = 0;
pub const OCEANIC: u8 = 1;

pub const CONVERGENT_CC: u8 = 0;
pub const CONVERGENT_SUBDUCTION: u8 = 1;
pub const CONVERGENT_OO: u8 = 2;
pub const DIVERGENT_CC: u8 = 3;
pub const DIVERGENT_OTHER: u8 = 4;
pub const TRANSFORM: u8 = 5;

/// Land target ~40%: the fixed sea-level percentile means the land fraction is
/// exact regardless of this number; this controls how FRAGMENTED that 40% is.
const FRACTION_CONTINENTAL: f64 = 0.45;
const TRANSFORM_RATIO: f64 = 0.55;

const HOTSPOT_OCEANIC_BIAS: f64 = 0.85;
const HOTSPOT_CHAIN_LINKS: i32 = 3;
const HOTSPOT_STEP_FRAC: f64 = 0.028;

// Height-field amplitudes (signed; only relative sizes matter — the whole
// field is min-max normalised downstream).
const BOUNDARY_FALLOFF_FRAC: f64 = 0.045;
const AMP_CONVERGENT_CC: f64 = 1.35;
const AMP_SUBDUCTION_RANGE: f64 = 1.05;
const AMP_SUBDUCTION_TRENCH: f64 = -0.85;
const AMP_CONVERGENT_OO: f64 = 0.22;
const AMP_DIVERGENT_CC: f64 = -0.65;
const AMP_DIVERGENT_OTHER: f64 = 0.06;
const BASE_CONTINENTAL: f64 = 0.90;
const BASE_OCEANIC: f64 = -0.90;
const HOTSPOT_BUMP_AMP: f64 = 0.9;
const HOTSPOT_BUMP_RADIUS_FRAC: f64 = 0.012;

pub struct Plate {
    pub kind: u8,
    pub cx: f64,
    pub cy: f64,
    pub drift_x: f64,
    pub drift_y: f64,
    pub speed: f64,
}

pub struct Boundary {
    pub x: i32,
    pub y: i32,
    pub plate_a: i32,
    pub plate_b: i32,
    pub kind: u8,
    pub strength: f64,
}

pub struct Plates {
    pub w: i32,
    pub h: i32,
    pub plate_id: Vec<i32>,
    pub plates: Vec<Plate>,
    pub boundaries: Vec<Boundary>,
    pub hotspot_chains: Vec<(i32, Vec<(f64, f64, f64)>)>, // (plate_id, [(x, y, strength)])
}

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

/// Wrap-aware squared distance in x between two cells, for seed placement.
fn wrap_dx(x: f64, px: f64, w: f64) -> f64 {
    (x - px + w / 2.0).rem_euclid(w) - w / 2.0
}

fn scatter_seeds(rng: &mut Rng, w: i32, h: i32, n: i32) -> Vec<(f64, f64)> {
    let mut placed: Vec<(f64, f64)> = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let mut best_score = -1.0f64;
        let mut best_xy = (0.0, 0.0);
        for _try in 0..60 {
            let x = rng.range_f64(0.0, w as f64);
            let y = rng.range_f64(0.0, h as f64);
            if placed.is_empty() {
                best_xy = (x, y);
                break;
            }
            let score = placed.iter().fold(f64::INFINITY, |acc, &(px, py)| {
                let dx = wrap_dx(x, px, w as f64);
                let dy = y - py;
                acc.min(dx * dx + dy * dy)
            });
            if score > best_score {
                best_score = score;
                best_xy = (x, y);
            }
        }
        placed.push(best_xy);
    }
    placed
}

fn grow_plate_ids(w: i32, h: i32, seeds: &[(f64, f64)], seed_val: i64) -> Vec<i32> {
    let spacing = w as f64 / (seeds.len() as f64).sqrt().max(1.0);
    let warp_amp = spacing * 0.35;
    let warp_oct = periodic_octaves(w, &[(0.010, 1.0), (0.024, 0.4)]);
    let wx = fbm_grid(w, h, seed_val + 501, &warp_oct, None, None);
    let wy = fbm_grid(w, h, seed_val + 907, &warp_oct, None, None);
    let (wxm, wym) = (wx.mean(), wy.mean());
    let mut wx = wx;
    let mut wy = wy;
    for v in wx.v.iter_mut() {
        *v = (*v - wxm) * 2.0 * warp_amp;
    }
    for v in wy.v.iter_mut() {
        *v = (*v - wym) * 2.0 * warp_amp;
    }

    let mut ids = vec![0i32; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let wxv = wx.get(x, y);
            let wyv = wy.get(x, y);
            let mut best = f64::INFINITY;
            let mut best_id = 0i32;
            for (i, &(sx, sy)) in seeds.iter().enumerate() {
                let dx = wrap_dx(x as f64 - sx + wxv, 0.0, w as f64);
                let dy = y as f64 - sy + wyv;
                let d2 = dx * dx + dy * dy;
                if d2 < best {
                    best = d2;
                    best_id = i as i32;
                }
            }
            ids[(y * w + x) as usize] = best_id;
        }
    }
    ids
}

fn classify_boundaries(w: i32, h: i32, plate_id: &[i32], plates: &[Plate]) -> Vec<Boundary> {
    let drift_x: Vec<f64> = plates.iter().map(|p| p.drift_x * p.speed).collect();
    let drift_y: Vec<f64> = plates.iter().map(|p| p.drift_y * p.speed).collect();
    let mut boundaries = Vec::new();

    for y in 0..h {
        for x in 0..w {
            let self_id = plate_id[(y * w + x) as usize];
            let mut normal_x = 0.0f64;
            let mut normal_y = 0.0f64;
            let mut other_id = -1i32;
            let mut is_boundary = false;
            for &(dx, dy) in &[(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let nx = (x + dx).rem_euclid(w);
                let ny = y + dy;
                if ny < 0 || ny >= h {
                    continue;
                }
                let nid = plate_id[(ny * w + nx) as usize];
                if nid != self_id {
                    normal_x += dx as f64;
                    normal_y += dy as f64;
                    if other_id < 0 {
                        other_id = nid;
                    }
                    is_boundary = true;
                }
            }
            if !is_boundary || other_id < 0 || self_id > other_id {
                continue; // keep the lower-id side only — one record per boundary point
            }

            let nlen = (normal_x * normal_x + normal_y * normal_y).sqrt().max(1.0);
            let (nx, ny) = (normal_x / nlen, normal_y / nlen);
            let (tx, ty) = (-ny, nx);
            let rel_x = drift_x[other_id as usize] - drift_x[self_id as usize];
            let rel_y = drift_y[other_id as usize] - drift_y[self_id as usize];
            let score_n = rel_x * nx + rel_y * ny; // negative = converging
            let score_t = rel_x * tx + rel_y * ty;
            let mag = (score_n * score_n + score_t * score_t).sqrt();
            let mag_safe = if mag == 0.0 { 1.0 } else { mag };
            let normal_frac = score_n.abs() / mag_safe;

            let ka = plates[self_id as usize].kind;
            let kb = plates[other_id as usize].kind;
            let both_cc = ka == CONTINENTAL && kb == CONTINENTAL;
            let both_oo = ka == OCEANIC && kb == OCEANIC;
            let kind = if normal_frac < TRANSFORM_RATIO {
                TRANSFORM
            } else if score_n < 0.0 {
                if both_cc {
                    CONVERGENT_CC
                } else if both_oo {
                    CONVERGENT_OO
                } else {
                    CONVERGENT_SUBDUCTION
                }
            } else if both_cc {
                DIVERGENT_CC
            } else {
                DIVERGENT_OTHER
            };

            boundaries.push(Boundary {
                x,
                y,
                plate_a: self_id,
                plate_b: other_id,
                kind,
                strength: mag,
            });
        }
    }
    boundaries
}

fn place_hotspots(rng: &mut Rng, plates: &[Plate], w: i32, n_hotspots: i32) -> Vec<(i32, Vec<(f64, f64, f64)>)> {
    let oceanic: Vec<usize> = plates
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == OCEANIC)
        .map(|(i, _)| i)
        .collect();
    let continental: Vec<usize> = plates
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == CONTINENTAL)
        .map(|(i, _)| i)
        .collect();
    let step = w as f64 * HOTSPOT_STEP_FRAC;
    let mut chains = Vec::new();
    for _ in 0..n_hotspots {
        let pool: &Vec<usize> = if !oceanic.is_empty() && rng.chance(HOTSPOT_OCEANIC_BIAS) {
            &oceanic
        } else if !continental.is_empty() {
            &continental
        } else if !oceanic.is_empty() {
            &oceanic
        } else {
            break;
        };
        let pi = pool[rng.below(pool.len())];
        let p = &plates[pi];
        let hx = p.cx + rng.range_f64(-0.3, 0.3) * w as f64;
        let hy = p.cy + rng.range_f64(-0.15, 0.15) * w as f64;
        let (back_x, back_y) = (-p.drift_x, -p.drift_y);
        let mut links = Vec::new();
        for age in 0..HOTSPOT_CHAIN_LINKS {
            let lx = (hx + back_x * step * age as f64).rem_euclid(w as f64);
            let ly = hy + back_y * step * age as f64;
            let strength = 1.0 - age as f64 / HOTSPOT_CHAIN_LINKS as f64;
            links.push((lx, ly, strength));
        }
        chains.push((pi as i32, links));
    }
    chains
}

pub fn generate_plates(w: i32, h: i32, seed: u64, n_plates: i32) -> Plates {
    let mut rng = Rng::new(seed);
    let seed_val = rng.next_u64() as i64;
    let n_hotspots = (n_plates / 8).max(1);

    let seeds = scatter_seeds(&mut rng, w, h, n_plates);
    let plate_id = grow_plate_ids(w, h, &seeds, seed_val);

    let n_continental = (n_plates as f64 * FRACTION_CONTINENTAL).round() as i32;
    let n_continental = n_continental.clamp(1, n_plates - 1);
    // A fixed COUNT of continental plates (not a per-plate coin flip), chosen
    // by shuffling the plate ids — see plates.py for why the count is pinned.
    let mut order: Vec<i32> = (0..n_plates).collect();
    for i in (1..order.len()).rev() {
        let j = rng.below(i + 1);
        order.swap(i, j);
    }
    let continental: std::collections::HashSet<i32> =
        order.iter().take(n_continental as usize).cloned().collect();

    let mut plates = Vec::with_capacity(n_plates as usize);
    for (i, &(sx, sy)) in seeds.iter().enumerate() {
        let kind = if continental.contains(&(i as i32)) {
            CONTINENTAL
        } else {
            OCEANIC
        };
        let angle = rng.range_f64(0.0, std::f64::consts::TAU);
        let speed = rng.range_f64(0.4, 1.0);
        plates.push(Plate {
            kind,
            cx: sx,
            cy: sy,
            drift_x: angle.cos(),
            drift_y: angle.sin(),
            speed,
        });
    }

    let boundaries = classify_boundaries(w, h, &plate_id, &plates);
    let hotspot_chains = place_hotspots(&mut rng, &plates, w, n_hotspots);

    Plates {
        w,
        h,
        plate_id,
        plates,
        boundaries,
        hotspot_chains,
    }
}

/// Multi-source 8-neighbour distance transform capped at `max_radius`, wrap in
/// x (cylinder topology), no y wrap. Exact BFS (cleaner than the Python's
/// iterative dilation, same character).
fn capped_distance(w: i32, h: i32, seeds: &[bool], max_radius: i32) -> Grid {
    let mut dist = Grid::new(w, h, max_radius as f64);
    let mut queue = std::collections::VecDeque::new();
    for y in 0..h {
        for x in 0..w {
            if seeds[(y * w + x) as usize] {
                dist.set(x, y, 0.0);
                queue.push_back((x, y));
            }
        }
    }
    while let Some((x, y)) = queue.pop_front() {
        let d = dist.get(x, y);
        if d >= max_radius as f64 {
            continue;
        }
        for &(dx, dy) in &NEIGH8 {
            let nx = (x + dx).rem_euclid(w);
            let ny = y + dy;
            if ny < 0 || ny >= h {
                continue;
            }
            if dist.get(nx, ny) > d + 1.0 {
                dist.set(nx, ny, d + 1.0);
                queue.push_back((nx, ny));
            }
        }
    }
    dist
}

fn falloff(dist: &Grid, max_radius: i32) -> Vec<f64> {
    dist.v
        .iter()
        .map(|&d| {
            let t = (1.0 - d / max_radius as f64).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        })
        .collect()
}

fn boundary_mask(boundaries: &[Boundary], kind: u8, w: i32, h: i32) -> Vec<bool> {
    let mut mask = vec![false; (w * h) as usize];
    for b in boundaries {
        if b.kind == kind {
            mask[(b.y * w + b.x) as usize] = true;
        }
    }
    mask
}

/// The plate-driven base of the height field, unnormalised (downstream does the
/// min-max + sea-level step). Composition per `plates.height_contribution`.
pub fn height_contribution(pl: &Plates) -> Grid {
    let (w, h) = (pl.w, pl.h);
    let max_radius = ((w as f64 * BOUNDARY_FALLOFF_FRAC).round() as i32).max(3);

    let own_continental: Vec<bool> = pl
        .plate_id
        .iter()
        .map(|&pid| pl.plates[pid as usize].kind == CONTINENTAL)
        .collect();

    let mut field = Grid::new(w, h, 0.0);
    for i in 0..field.v.len() {
        field.v[i] = if own_continental[i] {
            BASE_CONTINENTAL
        } else {
            BASE_OCEANIC
        };
    }

    let add = |field: &mut Grid, kind: u8, amp: f64, pl: &Plates, max_radius: i32| {
        let mask = boundary_mask(&pl.boundaries, kind, pl.w, pl.h);
        if mask.iter().any(|&b| b) {
            let dist = capped_distance(pl.w, pl.h, &mask, max_radius).blur(2, 2);
            let fo = falloff(&dist, max_radius);
            for i in 0..field.v.len() {
                field.v[i] += amp * fo[i];
            }
        }
    };

    add(&mut field, CONVERGENT_CC, AMP_CONVERGENT_CC, pl, max_radius);
    add(&mut field, CONVERGENT_OO, AMP_CONVERGENT_OO, pl, max_radius);
    add(&mut field, DIVERGENT_CC, AMP_DIVERGENT_CC, pl, max_radius);
    add(&mut field, DIVERGENT_OTHER, AMP_DIVERGENT_OTHER, pl, max_radius);

    // Subduction: asymmetric — coastal range on the continental side, trench on
    // the oceanic side of the same boundary.
    let sub_mask = boundary_mask(&pl.boundaries, CONVERGENT_SUBDUCTION, w, h);
    if sub_mask.iter().any(|&b| b) {
        let dist = capped_distance(w, h, &sub_mask, max_radius).blur(2, 2);
        let fo = falloff(&dist, max_radius);
        for i in 0..field.v.len() {
            let amp = if own_continental[i] {
                AMP_SUBDUCTION_RANGE
            } else {
                AMP_SUBDUCTION_TRENCH
            };
            field.v[i] += amp * fo[i];
        }
    }

    // Hotspot island chains: a radial bump per link, scaled by age.
    if !pl.hotspot_chains.is_empty() {
        let radius = (w as f64 * HOTSPOT_BUMP_RADIUS_FRAC).max(2.0);
        for (_pid, links) in &pl.hotspot_chains {
            for &(lx, ly, strength) in links {
                for y in 0..h {
                    for x in 0..w {
                        let dx = wrap_dx(x as f64, lx, w as f64);
                        let dy = y as f64 - ly;
                        let d2 = (dx * dx + dy * dy) / (radius * radius);
                        let bump = (1.0 - d2).max(0.0);
                        if bump > 0.0 {
                            field.set(x, y, field.get(x, y) + HOTSPOT_BUMP_AMP * strength * bump);
                        }
                    }
                }
            }
        }
    }

    field
}
