//! What a problem is made of: vehicles, the stops they make, and when.

use serde::{Deserialize, Serialize};

/// A place, in degrees.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
}

/// When something may happen, in seconds after midnight in the depot's
/// time zone. A soft window may be missed, at `penalty` per second late; a
/// hard one (no penalty) may not.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeWindow {
    pub start: u32,
    pub end: u32,
    pub penalty: Option<f64>,
}

impl TimeWindow {
    pub fn hard(start: u32, end: u32) -> Self {
        Self { start, end, penalty: None }
    }

    pub fn contains(&self, t: u32) -> bool {
        (self.start..=self.end).contains(&t)
    }
}

/// A customer to visit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Visit {
    pub id: String,
    pub location: Location,
    pub window: Option<TimeWindow>,
    /// Seconds spent there.
    pub service: u32,
    pub demand: Vec<u32>,
}

/// A van, its driver's shift and what it carries.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vehicle {
    pub id: String,
    pub depot: Location,
    pub shift: TimeWindow,
    pub capacity: Vec<u32>,
}

/// A stop on a route.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Stop {
    Depot(Location),
    Visit(Visit),
}

impl Stop {
    pub fn location(&self) -> Location {
        match self {
            Stop::Depot(l) => *l,
            Stop::Visit(v) => v.location,
        }
    }
}

/// Vehicles and the visits they share out.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Problem {
    pub vehicles: Vec<Vehicle>,
    pub visits: Vec<Visit>,
}
