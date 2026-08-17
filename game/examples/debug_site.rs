//! Debug: print both settlements' sites + year-by-year world state on a
//! fixed seed — the M3 economy (conversion, storage, spoilage) plus the M4
//! world (the second realm, the route, caravans, treasuries, storage builds).
use shapes_of_war::build::{self, BuildState, Treasury};
use shapes_of_war::economy;
use shapes_of_war::settlement::*;
use shapes_of_war::time::SimClock;
use shapes_of_war::trade::{self, TradeState};
use shapes_of_war::worldgen;

fn main() {
    let map = worldgen::generate(256, 160, 2024);
    let mut towns = vec![Settlement::spawn(&map, 2024)];
    towns.push(Settlement::spawn_at(&map, 2024, 1, 1, Some(towns[0].pos)));
    seed_initial_stockpiles(&mut towns);
    for s in &towns {
        println!(
            "site {} ({}, {}) climate {:?} fertility {:.3} pop {:.0} adults {:.0} max {:.0} tax {:.2}/d",
            s.name, s.pos.0, s.pos.1, s.climate, s.fertility_frac, s.population, s.adults,
            s.max_population, s.tax_income
        );
    }
    let mut trade_state = TradeState::new(&map, &towns[0], &towns[1]);
    let mut treasury = Treasury::new(2);
    let mut build_state = BuildState::default();

    let tick = 0.1;
    let mut seconds = 0.0;
    let mut last_year = 0i64;
    while seconds < 400.0 {
        for s in towns.iter_mut() {
            sim_tick(s, &SimClock { seconds }, tick);
        }
        trade::sim_tick(&mut trade_state, towns.iter_mut(), &mut treasury, &SimClock { seconds }, tick);
        build::sim_tick(&mut build_state, towns.iter_mut(), &mut treasury, tick);
        seconds += tick;
        let y = (seconds / 100.0).floor() as i64 + 1;
        if y != last_year {
            last_year = y;
            for s in &towns {
                let food: f64 = FOOD_SOURCES
                    .iter()
                    .map(|r| s.resources.get(*r).copied().unwrap_or(0.0))
                    .sum();
                let storage: f64 = s
                    .resources
                    .iter()
                    .map(|(r, v)| v * economy::bulk(r))
                    .sum();
                let route = if !trade_state.route_projects.is_empty() {
                    let p = &trade_state.route_projects[0];
                    format!("route {:.0}%", 100.0 * p.progress / p.total_cells())
                } else if !trade_state.routes.is_empty() {
                    "route open".to_string()
                } else {
                    "no route".to_string()
                };
                println!(
                    "year {} day {:.0} [{}] pop {:.0} adults {:.0} hunger {:.1}d cold {:.1}d food {:.1} fw {:.1} logs {:.1} stone {:.1} iron {:.1} gold {:.1} treasury {:.0} storage {:.0} | {} | {} caravan(s) | {}",
                    y,
                    seconds,
                    s.name,
                    s.population,
                    s.adults,
                    s.days_without_food,
                    s.days_without_firewood,
                    food,
                    s.resources.get("Firewood").copied().unwrap_or(0.0),
                    s.resources.get("Logs").copied().unwrap_or(0.0),
                    s.resources.get("Stone").copied().unwrap_or(0.0),
                    s.resources.get("Iron").copied().unwrap_or(0.0),
                    s.resources.get("Gold").copied().unwrap_or(0.0),
                    treasury.gold[s.faction_idx],
                    storage,
                    route,
                    trade_state.caravans.len(),
                    if build_state.projects.is_empty() {
                        "".to_string()
                    } else {
                        format!(
                            "building {} tier {}",
                            build_state.projects[0].building.name(),
                            build_state.projects[0].to_tier
                        )
                    }
                );
            }
        }
    }
}
