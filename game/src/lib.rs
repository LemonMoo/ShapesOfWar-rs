//! Shapes of War — the continuous-time 4X core (library target).
//!
//! `main.rs` is a thin binary over this library; the headless fingerprint
//! tests and examples import the modules from here exactly the way the
//! Python project's `dev/` scripts imported the `app.world` package.

pub mod build;
pub mod economy;
pub mod grid;
pub mod noise;
pub mod plates;
pub mod rng;
pub mod settlement;
pub mod time;
pub mod trade;
pub mod war;
pub mod worldgen;
