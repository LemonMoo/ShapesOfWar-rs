//! A 2-D grid of f64, the shared working type for worldgen. Flat `Vec` (not
//! `Vec<Vec<..>>`) so a whole layer is one allocation and cache-friendly.

#[derive(Clone)]
pub struct Grid {
    pub w: i32,
    pub h: i32,
    pub v: Vec<f64>,
}

impl Grid {
    pub fn new(w: i32, h: i32, fill: f64) -> Self {
        Grid {
            w,
            h,
            v: vec![fill; (w as usize) * (h as usize)],
        }
    }

    #[inline]
    pub fn idx(&self, x: i32, y: i32) -> usize {
        (y * self.w + x) as usize
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> f64 {
        self.v[self.idx(x, y)]
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, val: f64) {
        let i = self.idx(x, y);
        self.v[i] = val;
    }

    pub fn mean(&self) -> f64 {
        self.v.iter().sum::<f64>() / self.v.len() as f64
    }

    pub fn min(&self) -> f64 {
        self.v.iter().cloned().fold(f64::INFINITY, f64::min)
    }

    pub fn max(&self) -> f64 {
        self.v.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
    }

    /// Box-blur (wrap in x) — turns a discrete step field (e.g. an integer
    /// BFS distance) into a smooth gradient, so a smoothstep over it no longer
    /// produces concentric contour bands.
    pub fn blur(&self, radius: i32, passes: i32) -> Grid {
        let (w, h) = (self.w, self.h);
        let mut src = self.clone();
        let mut dst = Grid::new(w, h, 0.0);
        for _ in 0..passes {
            for y in 0..h {
                for x in 0..w {
                    let mut sum = 0.0;
                    let mut cnt = 0.0;
                    for dy in -radius..=radius {
                        for dx in -radius..=radius {
                            let nx = (x + dx).rem_euclid(w);
                            let ny = y + dy;
                            if ny < 0 || ny >= h {
                                continue;
                            }
                            sum += src.get(nx, ny);
                            cnt += 1.0;
                        }
                    }
                    dst.set(x, y, sum / cnt);
                }
            }
            std::mem::swap(&mut src, &mut dst);
        }
        src
    }
}

/// A grid of booleans (land masks, boundary masks).
#[derive(Clone)]
pub struct BoolGrid {
    pub w: i32,
    pub h: i32,
    pub v: Vec<bool>,
}

impl BoolGrid {
    pub fn new(w: i32, h: i32, fill: bool) -> Self {
        BoolGrid {
            w,
            h,
            v: vec![fill; (w as usize) * (h as usize)],
        }
    }

    #[inline]
    pub fn idx(&self, x: i32, y: i32) -> usize {
        (y * self.w + x) as usize
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> bool {
        self.v[self.idx(x, y)]
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, val: bool) {
        let i = self.idx(x, y);
        self.v[i] = val;
    }
}
