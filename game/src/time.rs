//! `time` — the continuous clock (port milestone M2).
//!
//! The Python game's "day" is an atomic turn sliced across frames; this port
//! replaces it with a real continuous clock. `SimClock` is a single f64 count
//! of sim-seconds, advanced at a fixed 10 Hz timestep, and everything the old
//! turn counter used to own — season, year, day/night — becomes a *derived
//! function of the clock* instead of a counter that has to be rolled.
//!
//! Time scale: **1 sim-second = 1 day**, matching the Python constants it
//! ports (25 days/season, 100 days/year — `resources.py: TURNS_PER_SEASON`
//! / `YEAR_LENGTH_TURNS`). Sim time runs at real-time pace (10 fixed ticks
//! per real second × 0.1 s), so a year takes ~100 real seconds — the same
//! pacing the Python game already had at ~1 turn/second. The world starts in
//! Spring at `seconds = 0`, exactly like `world.season = "Spring"` does.

use bevy::prelude::*;

/// Fixed sim ticks per real second — the integration resolution, not a
/// time-dilation factor (1 sim-second = 1 real second).
pub const SIM_HZ: f64 = 10.0;
/// Days per season (Python `resources.py: TURNS_PER_SEASON`).
pub const TURNS_PER_SEASON: f64 = 25.0;
/// Days per year (Python `YEAR_LENGTH_TURNS`).
pub const YEAR_LENGTH_DAYS: f64 = 100.0;

/// The four seasons, in Python `resources.py: SEASONS` order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

impl Season {
    pub fn from_idx(i: usize) -> Season {
        match i % 4 {
            0 => Season::Spring,
            1 => Season::Summer,
            2 => Season::Autumn,
            _ => Season::Winter,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Season::Spring => "Spring",
            Season::Summer => "Summer",
            Season::Autumn => "Autumn",
            Season::Winter => "Winter",
        }
    }
}

/// The continuous clock. `seconds` counts sim-days since the world began.
#[derive(Resource)]
pub struct SimClock {
    pub seconds: f64,
}

impl SimClock {
    pub fn new() -> Self {
        SimClock { seconds: 0.0 }
    }

    /// Day number, 1-based (the Python world's `world.turn` equivalent).
    pub fn day(&self) -> i64 {
        self.seconds.floor() as i64 + 1
    }

    /// Day within the current year, 1-based (1..=100).
    pub fn day_of_year(&self) -> i64 {
        (self.seconds % YEAR_LENGTH_DAYS).floor() as i64 + 1
    }

    /// Year number, 1-based.
    pub fn year(&self) -> i64 {
        (self.seconds / YEAR_LENGTH_DAYS).floor() as i64 + 1
    }

    /// Current season — `SEASONS[(turn-1) // TURNS_PER_SEASON % 4]`, in
    /// continuous form. `seconds=0` (the start of day 1) is Spring.
    pub fn season(&self) -> Season {
        Season::from_idx((self.seconds / TURNS_PER_SEASON).floor() as i64 as usize)
    }

    /// Days left in the current season (fractional).
    pub fn days_left_in_season(&self) -> f64 {
        TURNS_PER_SEASON - (self.seconds % TURNS_PER_SEASON)
    }

    /// 0..1 phase of the day; 0.25..0.75 is daylight (derived day/night).
    pub fn day_phase(&self) -> f64 {
        self.seconds % 1.0
    }

    pub fn is_day(&self) -> bool {
        let p = self.day_phase();
        (0.25..0.75).contains(&p)
    }
}

pub struct TimePlugin;

impl Plugin for TimePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(SIM_HZ))
            .insert_resource(SimClock::new())
            .add_systems(FixedUpdate, advance_clock);
    }
}

/// Advance the sim clock by the fixed timestep. Inside `FixedUpdate`,
/// `Time<Fixed>::delta_secs()` is exactly the fixed step (0.1 s = 0.1 day),
/// so the sim is a deterministic function of the number of ticks, exactly as
/// the port plan demands.
fn advance_clock(mut clock: ResMut<SimClock>, time: Res<Time<Fixed>>) {
    clock.seconds += time.delta_secs_f64();
}
