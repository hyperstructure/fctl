//! Random problems for the property tests: a depot in Stockholm, vehicles
//! with morning shifts, visits within 15 km.

use milkfloat::model::{Location, Problem, Stop, TimeWindow, Vehicle, Visit};
use milkfloat::route::Route;
use proptest::prelude::*;
use std::ops::Range;

const DEPOT: Location = Location { lat: 59.3326, lon: 18.0649 };

pub fn problem(vehicles: Range<usize>, visits: Range<usize>) -> impl Strategy<Value = Problem> {
    (prop::collection::vec(vehicle(), vehicles), prop::collection::vec(visit(), visits)).prop_map(|(vehicles, mut visits)| {
        for (i, v) in visits.iter_mut().enumerate() {
            v.id = format!("v{i}");
        }
        Problem { vehicles, visits }
    })
}

fn vehicle() -> impl Strategy<Value = Vehicle> {
    (5u32..8, 6u32..10, 20u32..60).prop_map(|(start, hours, capacity)| Vehicle {
        id: "van".into(),
        depot: DEPOT,
        shift: TimeWindow::hard(start * 3600, (start + hours) * 3600),
        capacity: vec![capacity],
    })
}

fn visit() -> impl Strategy<Value = Visit> {
    (-0.1f64..0.1, -0.15f64..0.15, prop::option::of(6u32..12), 60u32..600, 1u32..10).prop_map(|(dlat, dlon, hour, service, demand)| Visit {
        id: String::new(),
        location: Location { lat: DEPOT.lat + dlat, lon: DEPOT.lon + dlon },
        window: hour.map(|h| TimeWindow::hard(h * 3600, h * 3600 + 7200)),
        service,
        demand: vec![demand],
    })
}

pub fn visit_ids(route: &Route) -> Vec<String> {
    route
        .stops
        .iter()
        .filter_map(|s| match s {
            Stop::Visit(v) => Some(v.id.clone()),
            _ => None,
        })
        .collect()
}

pub fn keeps_hard_windows(route: &Route) -> bool {
    route.stops.iter().zip(&route.arrivals).all(|(s, at)| match s {
        Stop::Visit(Visit { window: Some(w), .. }) if w.penalty.is_none() => *at <= w.end,
        _ => true,
    })
}
