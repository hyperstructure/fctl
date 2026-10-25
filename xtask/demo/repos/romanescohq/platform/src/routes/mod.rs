//! Delivery routes: the day's stops, the vans that take them, and the
//! planner that puts one in order for the other.

pub mod matrix;
pub mod plan;

use chrono::{DateTime, Utc};

#[derive(Clone, Debug)]
pub struct Van {
    pub id: i64,
    /// Boxes it holds.
    pub capacity: u32,
    /// The driver's shift, by the depot's clock.
    pub shift_start: chrono::NaiveTime,
    pub shift_end: chrono::NaiveTime,
}

#[derive(Clone, Debug)]
pub struct Stop {
    pub id: i64,
    pub address: String,
    pub location: (f64, f64),
    pub boxes: u32,
    /// The window the customer chose (0137).
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
}

#[derive(Clone, Debug, Default)]
pub struct Route {
    pub van: i64,
    pub stops: Vec<Stop>,
}

impl Route {
    pub fn new(van: i64) -> Self {
        Self { van, stops: Vec::new() }
    }

    /// The boxes on board when it leaves the depot.
    pub fn load(&self) -> u32 {
        self.stops.iter().map(|s| s.boxes).sum()
    }
}

/// `POST /v1/routes/{day}`: plans the day's routes for every depot, after
/// the 18:00 cutoff the evening before.
pub async fn plan_day(
    axum::extract::State(db): axum::extract::State<crate::db::Db>,
    axum::extract::Path(day): axum::extract::Path<chrono::NaiveDate>,
) -> Result<axum::Json<Vec<Route>>, crate::Error> {
    let mut all = Vec::new();
    for depot in db.depots().await? {
        let stops = db.stops(&depot.id, day).await?;
        let matrix = matrix::build(&db.routing(), &stops).await?;
        all.extend(plan::plan(&depot, &stops, &matrix));
    }
    Ok(axum::Json(all))
}
