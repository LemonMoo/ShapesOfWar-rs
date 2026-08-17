//! Value noise + FBM, a scalar port of `app/world/noise.py` (which is itself
//! the vectorised twin of `worldgen._vhash`/`_vnoise`). Deterministic and
//! periodic in x so the map's east-west wrap never shows a seam.

use crate::grid::Grid;

/// The integer hash behind the noise. Bit-for-bit the same mixing as
/// `noise.vhash_np` / `worldgen._vhash`, so the noise character matches.
#[inline]
fn hash(ix: i64, iy: i64, seed: i64) -> f64 {
    let mut n = (ix.wrapping_mul(73_856_093))
        ^ (iy.wrapping_mul(19_349_663))
        ^ (seed.wrapping_mul(83_492_791));
    n &= 0xFFFF_FFFF;
    n = ((n ^ (n >> 13)).wrapping_mul(1_274_126_177)) & 0xFFFF_FFFF;
    n ^= n >> 16;
    (n & 0xFFFF) as f64 / 0xFFFF as f64
}

/// The raw hash in [0,1) — `worldgen._vhash` — used for deterministic
/// tie-breaking (e.g. river direction choice) rather than interpolated noise.
#[inline]
pub fn vhash(ix: i64, iy: i64, seed: i64, period_x: Option<i64>) -> f64 {
    let ix = match period_x {
        Some(p) => ix.rem_euclid(p),
        None => ix,
    };
    hash(ix, iy, seed)
}

/// 2-D value noise at a continuous coordinate, smoothstep-interpolated.
/// `period_x`, when given, wraps the x lattice so the field is periodic.
pub fn value_noise(x: f64, y: f64, seed: i64, period_x: Option<i64>) -> f64 {
    let sx = x.floor();
    let sy = y.floor();
    let mut fx = x - sx;
    let mut fy = y - sy;
    fx = fx * fx * (3.0 - 2.0 * fx);
    fy = fy * fy * (3.0 - 2.0 * fy);
    let ix0 = sx as i64;
    let iy0 = sy as i64;
    let wrap = |ix: i64| match period_x {
        Some(p) => ix.rem_euclid(p),
        None => ix,
    };
    let v00 = hash(wrap(ix0), iy0, seed);
    let v10 = hash(wrap(ix0 + 1), iy0, seed);
    let v01 = hash(wrap(ix0), iy0 + 1, seed);
    let v11 = hash(wrap(ix0 + 1), iy0 + 1, seed);
    let a = v00 + (v10 - v00) * fx;
    let b = v01 + (v11 - v01) * fx;
    a + (b - a) * fy
}

/// Precompute `(eff_freq, period_cells, amp)` per `(freq, amp)` octave —
/// mirrors `worldgen._periodic_octaves`, so a frequency is an integer lattice
/// period in x and the field is seamless at the wrap.
pub fn periodic_octaves(width: i32, octaves: &[(f64, f64)]) -> Vec<(f64, i64, f64)> {
    octaves
        .iter()
        .map(|&(freq, amp)| {
            let period = ((width as f64 * freq).round()).max(1.0) as i64;
            let eff = period as f64 / width as f64;
            (eff, period, amp)
        })
        .collect()
}

/// FBM over the whole grid. `warp_x`/`warp_y`, when given, are per-cell offsets
/// in CELL units added to the sample coordinate before scaling by frequency —
/// domain warping, which is what turns isotropic value noise into twisted,
/// elongated detail (see `worldgen.py`'s height-field comment).
pub fn fbm_grid(
    w: i32,
    h: i32,
    seed: i64,
    octaves: &[(f64, i64, f64)],
    warp_x: Option<&Grid>,
    warp_y: Option<&Grid>,
) -> Grid {
    let mut out = Grid::new(w, h, 0.0);
    for y in 0..h {
        for x in 0..w {
            let wx = warp_x.map_or(0.0, |g| g.get(x, y));
            let wy = warp_y.map_or(0.0, |g| g.get(x, y));
            let mut total = 0.0;
            for &(eff, period, amp) in octaves {
                total += amp
                    * value_noise(
                        (x as f64 + wx) * eff,
                        (y as f64 + wy) * eff,
                        seed,
                        Some(period),
                    );
            }
            out.set(x, y, total);
        }
    }
    out
}
