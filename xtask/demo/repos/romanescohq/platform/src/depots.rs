//! Depots: where the vans leave from, each in its own time zone.

use chrono::{DateTime, Timelike, Utc};

#[derive(Clone, Debug)]
pub struct Depot {
    /// `sto-1`, `cph-1`.
    pub id: String,
    pub timezone: chrono_tz::Tz,
    pub vans: Vec<crate::routes::Van>,
}

impl Depot {
    /// Seconds after midnight at the depot, as the drivers' app shows times.
    pub fn local_seconds(&self, t: DateTime<Utc>) -> i64 {
        let local = t.with_timezone(&self.timezone);
        i64::from(local.time().num_seconds_from_midnight())
    }
}
