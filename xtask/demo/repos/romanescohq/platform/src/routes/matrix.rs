//! The distance matrix between a day's stops: travel times by road, from
//! the routing service, for the planner.

use super::Stop;
use milkfloat::Matrix;

/// Travel times in seconds between every pair of `stops`, a row at a time:
/// the routing service takes one origin and many destinations a request.
pub async fn build(router: &crate::routing::Client, stops: &[Stop]) -> Result<Matrix, crate::Error> {
    let mut matrix = Matrix::new(stops.len());
    for (i, from) in stops.iter().enumerate() {
        let row = router
            .travel_times(from.location, stops.iter().map(|s| s.location))
            .await?;
        matrix.set_row(i, &row);
    }
    Ok(matrix)
}
