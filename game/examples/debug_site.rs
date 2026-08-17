//! Debug: print the settlement's site + year-by-year state on a fixed seed,
//! now including the M3 economy (conversion, storage, spoilage).
use shapes_of_war::economy;
use shapes_of_war::settlement::*;
use shapes_of_war::time::{Season, SimClock};
use shapes_of_war::worldgen;

fn main() {
    let map = worldgen::generate(256, 160, 2024);
    let mut s = Settlement::spawn(&map, 2024);
    println!(
        "site ({}, {}) climate {:?} fertility {:.3} pop {:.0} adults {:.0} max {:.0}",
        s.pos.0, s.pos.1, s.climate, s.fertility_frac, s.population, s.adults, s.max_population
    );
    let mut b: Vec<String> = s.biome_counts.iter().map(|(k, v)| format!("{k}x{v}")).collect();
    b.sort();
    println!("biomes: {}", b.join(" "));
    for season in [Season::Spring, Season::Summer, Season::Autumn, Season::Winter] {
        let rates = production_rates(&s, season);
        let mut r: Vec<String> = rates.iter().map(|(k, v)| format!("{k}:{v:.1}")).collect();
        r.sort();
        println!("{season:?} production: {}", r.join(" "));
    }
    let tick = 0.1;
    let mut seconds = 0.0;
    let mut last_year = 0i64;
    while seconds < 400.0 {
        sim_tick(&mut s, &SimClock { seconds }, tick);
        seconds += tick;
        let y = (seconds / 100.0).floor() as i64 + 1;
        if y != last_year {
            last_year = y;
            let food: f64 = FOOD_SOURCES
                .iter()
                .map(|r| s.resources.get(*r).copied().unwrap_or(0.0))
                .sum();
            let storage: f64 = s
                .resources
                .iter()
                .map(|(r, v)| v * economy::bulk(r))
                .sum();
            let mut conv: Vec<String> = economy::conversion_rates(&s.resources, seconds)
                .iter()
                .map(|(o, d)| format!("{o}:{d:.1}/d"))
                .collect();
            conv.sort();
            println!(
                "year {} day {:.0}: pop {:.0} adults {:.0} hunger {:.1}d cold {:.1}d food {:.1} fw {:.1} logs {:.1} planks {:.1} clothes {:.1} wine {:.1} storage {:.0} | converting: {}",
                y,
                seconds,
                s.population,
                s.adults,
                s.days_without_food,
                s.days_without_firewood,
                food,
                s.resources.get("Firewood").copied().unwrap_or(0.0),
                s.resources.get("Logs").copied().unwrap_or(0.0),
                s.resources.get("Planks").copied().unwrap_or(0.0),
                s.resources.get("Clothes").copied().unwrap_or(0.0),
                s.resources.get("Wine").copied().unwrap_or(0.0),
                storage,
                if conv.is_empty() {
                    "—".to_string()
                } else {
                    conv.join(" ")
                }
            );
        }
    }
}
