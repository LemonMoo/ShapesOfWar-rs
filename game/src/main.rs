//! Shapes of War — Rust core, milestone 1: natural world generation.
//!
//! The walking skeleton's sine-blob map is gone. `worldgen::generate` runs the
//! full terrain pipeline (tectonic plates → domain-warped elevation → sea
//! level → erosion → rivers/lakes → climate → biome) and this renders it as a
//! single relief-shaded (hillshade + coastal shelf) image — the 2.5D direction
//! made visible, with elevation as a first-class field.

mod grid;
mod noise;
mod plates;
mod rng;
mod worldgen;

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

const MAP_W: i32 = 512;
const MAP_H: i32 = 320;
const SEED: u64 = 2024;

/// The generated world, wrapped so the pure `worldgen` module stays Bevy-free.
#[derive(Resource)]
struct Terrain(worldgen::WorldMap);

fn biome_base(b: u8) -> (f64, f64, f64) {
    match b {
        worldgen::BIOME_ICE => (0.93, 0.95, 0.98),
        worldgen::BIOME_TUNDRA => (0.74, 0.77, 0.68),
        worldgen::BIOME_ALPINE => (0.70, 0.68, 0.64),
        worldgen::BIOME_TAIGA => (0.22, 0.40, 0.28),
        worldgen::BIOME_BOREAL => (0.14, 0.32, 0.22),
        worldgen::BIOME_STEPPE => (0.74, 0.68, 0.40),
        worldgen::BIOME_GRASSLAND => (0.60, 0.71, 0.34),
        worldgen::BIOME_PLAINS => (0.48, 0.66, 0.30),
        worldgen::BIOME_TEMPERATE_FOREST => (0.22, 0.45, 0.20),
        worldgen::BIOME_TEMPERATE_RAINFOREST => (0.13, 0.38, 0.22),
        worldgen::BIOME_SHRUBLAND => (0.70, 0.66, 0.42),
        worldgen::BIOME_DESERT => (0.87, 0.78, 0.52),
        worldgen::BIOME_SAVANNAH => (0.76, 0.72, 0.36),
        worldgen::BIOME_MONSOON => (0.27, 0.51, 0.24),
        worldgen::BIOME_JUNGLE => (0.11, 0.39, 0.13),
        worldgen::BIOME_MOUNTAIN => (0.55, 0.52, 0.50),
        worldgen::BIOME_SNOW_PEAK => (0.96, 0.97, 0.99),
        worldgen::BIOME_HIGHLAND => (0.62, 0.56, 0.44),
        worldgen::BIOME_COASTAL => (0.84, 0.80, 0.64),
        worldgen::BIOME_SWAMP => (0.26, 0.38, 0.26),
        worldgen::BIOME_MARSH => (0.40, 0.48, 0.38),
        worldgen::BIOME_MANGROVE => (0.18, 0.40, 0.28),
        _ => (0.5, 0.5, 0.5),
    }
}

/// Diffuse hillshade from a light to the north-west: relief lighting that turns
/// a flat biome colour into readable terrain. `ex` is vertical exaggeration.
fn hillshade(map: &worldgen::WorldMap, x: i32, y: i32, ex: f64) -> f64 {
    let xl = (x - 1).rem_euclid(map.w);
    let xr = (x + 1).rem_euclid(map.w);
    let yu = (y - 1).clamp(0, map.h - 1);
    let yd = (y + 1).clamp(0, map.h - 1);
    let dzdx = map.height.get(xr, y) - map.height.get(xl, y);
    let dzdy = map.height.get(x, yd) - map.height.get(x, yu);
    let (nx, ny, nz) = (-dzdx * ex, -dzdy * ex, 1.0);
    let len = (nx * nx + ny * ny + nz * nz).sqrt();
    let (nx, ny, nz) = (nx / len, ny / len, nz / len);
    // direction to the light (north-west, above), normalised
    let (lx, ly, lz) = (-0.5, -0.5, 0.7071);
    let d = (nx * lx + ny * ly + nz * lz).max(0.0);
    0.55 + 0.45 * d
}

fn pixel(map: &worldgen::WorldMap, x: i32, y: i32) -> (f64, f64, f64) {
    let i = (y * map.w + x) as usize;

    if map.lake.get(x, y) {
        // Freshwater — the same blue-green family as shallow ocean, so inland
        // water reads as water rather than an unrelated colour.
        return (0.28, 0.56, 0.70);
    }
    if !map.land.get(x, y) {
        // Ocean: continuous depth below sea level, smoothstepped into a
        // shallow-turquoise → deep-navy ramp. The depth field is the elevation
        // itself, so there are no discrete contour bands in the shallows.
        let t = (map.ocean_depth.get(x, y) / map.sea_level).clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        return (0.34 - 0.31 * t, 0.63 - 0.55 * t, 0.74 - 0.56 * t);
    }
    if map.river.get(x, y) {
        return (0.30, 0.54, 0.70);
    }
    let (r, g, b) = biome_base(map.biome[i]);
    let sh = hillshade(map, x, y, 4.0);
    (r * sh, g * sh, b * sh)
}

fn main() {
    let map = worldgen::generate(MAP_W, MAP_H, SEED);

    println!(
        "worldgen: {}x{}  land={:.1}%  continents={}  river_cells={}  sea_level={:.3}",
        map.w,
        map.h,
        100.0 * map.n_land as f64 / (map.w * map.h) as f64,
        map.n_continents,
        map.n_river_cells,
        map.sea_level
    );
    let mut hist: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for &b in &map.biome {
        *hist.entry(worldgen::biome_name(b)).or_insert(0) += 1;
    }
    let s: Vec<String> = hist.iter().map(|(k, v)| format!("{k}:{v}")).collect();
    println!("biomes: {}", s.join("  "));

    App::new()
        .add_plugins(DefaultPlugins)
        .insert_resource(Terrain(map))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>, terrain: Res<Terrain>) {
    let map = &terrain.0;
    let (w, h) = (map.w, map.h);
    let mut data = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = pixel(map, x, y);
            let i = ((y * w + x) * 4) as usize;
            data[i] = (r.clamp(0.0, 1.0) * 255.0) as u8;
            data[i + 1] = (g.clamp(0.0, 1.0) * 255.0) as u8;
            data[i + 2] = (b.clamp(0.0, 1.0) * 255.0) as u8;
            data[i + 3] = 255;
        }
    }
    let image = Image::new(
        Extent3d {
            width: w as u32,
            height: h as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    );
    let handle = images.add(image);

    commands.spawn(Camera2d);
    commands.spawn((
        Sprite {
            image: handle,
            custom_size: Some(Vec2::new(w as f32, h as f32)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
}
