//! Routes as GeoJSON: a LineString for each route and a Point for each stop.

use crate::model::Stop;
use crate::route::{Route, geometry};
use serde_json::{Value, json};

/// One route as a FeatureCollection.
pub fn route(route: &Route) -> Value {
    let line: Vec<[f64; 2]> = geometry::line(route).iter().map(|l| [l.lon, l.lat]).collect();
    let mut features = vec![json!({
        "type": "Feature",
        "geometry": { "type": "LineString", "coordinates": line },
        "properties": { "vehicle": route.vehicle.id },
    })];
    features.extend(route.stops.iter().zip(&route.arrivals).map(|(stop, at)| {
        let l = stop.location();
        json!({
            "type": "Feature",
            "geometry": { "type": "Point", "coordinates": [l.lon, l.lat] },
            "properties": { "stop": name(stop), "arrival": at },
        })
    }));
    json!({ "type": "FeatureCollection", "features": features })
}

fn name(stop: &Stop) -> String {
    match stop {
        Stop::Depot(_) => "depot".to_owned(),
        Stop::Visit(v) => v.id.clone(),
    }
}
