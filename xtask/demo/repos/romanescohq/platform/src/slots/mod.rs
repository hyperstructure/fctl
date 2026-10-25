//! Delivery slots: the days and windows a postcode can get a box on.

use crate::db::Db;
use axum::Json;
use axum::extract::{Path, State};

#[derive(Clone, Debug, serde::Serialize)]
pub struct Slot {
    pub day: chrono::NaiveDate,
    pub window_start: chrono::NaiveTime,
    pub window_end: chrono::NaiveTime,
    pub farms: Vec<String>,
    pub full: bool,
}

/// `GET /v1/slots/{postcode}`: the next three weeks' slots, for the shop's
/// delivery-day picker.
pub async fn for_postcode(
    State(db): State<Db>,
    Path(postcode): Path<String>,
) -> Result<Json<Vec<Slot>>, crate::Error> {
    Ok(Json(query(&db, &postcode).await?))
}

/// The slots, from the routes, the vans' loads and the farms' picking days.
pub async fn query(db: &Db, postcode: &str) -> Result<Vec<Slot>, crate::Error> {
    let slots = sqlx::query_as!(
        Slot,
        r#"SELECT d.day, d.window_start, d.window_end,
                  array_agg(DISTINCT f.name) AS "farms!", bool_or(r.load >= r.capacity) AS "full!"
             FROM delivery_days d
             JOIN zone_postcodes z ON z.zone = d.zone
             JOIN routes r ON r.day = d.day AND r.zone = d.zone
             JOIN farm_picking p ON p.weekday = extract(isodow FROM d.day)
             JOIN farms f ON f.id = p.farm_id AND f.zone = d.zone
            WHERE z.postcode = $1 AND d.day BETWEEN current_date AND current_date + 21
            GROUP BY d.day, d.window_start, d.window_end
            ORDER BY d.day, d.window_start"#,
        postcode
    )
    .fetch_all(db.pool())
    .await?;
    Ok(slots)
}
