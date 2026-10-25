//! The vans' routes for a delivery day: nearest neighbour from the depot.

pub fn plan(stops: &[crate::Stop]) -> Vec<crate::Route> {
    crate::routes::nearest_neighbour(stops)
}
