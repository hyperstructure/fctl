//! A route's line on a map: its stops' locations in order.

use crate::model::Location;
use crate::route::Route;

/// The points a route passes, depot to depot.
pub fn line(route: &Route) -> Vec<Location> {
    route.stops.iter().map(|s| s.location()).collect()
}

/// Its length in metres, as the crow flies between stops.
pub fn crow_length(route: &Route) -> f64 {
    line(route).windows(2).map(|w| haversine(w[0], w[1])).sum()
}

fn haversine(a: Location, b: Location) -> f64 {
    const R: f64 = 6_371_000.0;
    let (la, lb) = (a.lat.to_radians(), b.lat.to_radians());
    let dl = (b.lat - a.lat).to_radians();
    let dn = (b.lon - a.lon).to_radians();
    let h = (dl / 2.0).sin().powi(2) + la.cos() * lb.cos() * (dn / 2.0).sin().powi(2);
    2.0 * R * h.sqrt().asin()
}
