//! Cheapest insertion: puts each visit where it adds least to a route,
//! without breaking a hard window, a capacity or a shift.

use crate::model::{Stop, Visit};
use crate::route::Route;

/// Why something can't go on a route.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum Infeasible {
    #[error("the vehicle has no room for it")]
    Capacity,
    #[error("no place on the route meets {visit}'s time window")]
    Window { visit: String },
    #[error("it would run past the end of the shift")]
    Shift,
}

/// Inserts `visit` where it adds least to `route`'s cost.
pub fn insert_visit(route: &mut Route, visit: &Visit, cost: impl Fn(&Route, usize) -> Option<f64>) -> Result<(), Infeasible> {
    if !fits(route, visit) {
        return Err(Infeasible::Capacity);
    }
    let best = (1..route.stops.len())
        .filter_map(|i| cost(route, i).map(|c| (i, c)))
        .min_by(|a, b| a.1.total_cmp(&b.1));
    match best {
        Some((i, _)) => {
            route.stops.insert(i, Stop::Visit(visit.clone()));
            Ok(())
        }
        None => Err(Infeasible::Window { visit: visit.id.clone() }),
    }
}

fn fits(route: &Route, visit: &Visit) -> bool {
    route.vehicle.capacity.iter().enumerate().all(|(d, cap)| {
        let used: u32 = route
            .stops
            .iter()
            .filter_map(|s| match s {
                Stop::Visit(v) => v.demand.get(d).copied(),
                Stop::Depot(_) => None,
            })
            .sum();
        used + visit.demand.get(d).copied().unwrap_or(0) <= *cap
    })
}
