//! The vans' routes for a delivery day: nearest neighbour from the depot.

use super::{Route, Stop};
use crate::depots::Depot;
use milkfloat::Matrix;

/// Plans `depot`'s routes: each van takes the nearest stop it hasn't been
/// to until it's full, then goes back.
pub fn plan(depot: &Depot, stops: &[Stop], matrix: &Matrix) -> Vec<Route> {
    let mut left: Vec<usize> = (0..stops.len()).collect();
    let mut routes = Vec::new();
    for van in &depot.vans {
        let mut route = Route::new(van.id);
        let mut at = None;
        while let Some(next) = nearest(matrix, at, &left) {
            if route.load() + stops[next].boxes > van.capacity {
                break;
            }
            route.stops.push(stops[next].clone());
            left.retain(|&s| s != next);
            at = Some(next);
        }
        routes.push(route);
    }
    routes
}

/// The stop of `left` nearest to `from`, by travel time; from the depot,
/// the first.
fn nearest(matrix: &Matrix, from: Option<usize>, left: &[usize]) -> Option<usize> {
    left.iter()
        .copied()
        .min_by_key(|&s| from.map_or(0, |f| matrix.get(f, s)))
}
