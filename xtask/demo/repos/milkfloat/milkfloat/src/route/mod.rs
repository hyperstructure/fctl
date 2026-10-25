//! Routes: the stops a vehicle makes, in order, and when it gets to each.

pub mod geometry;

use crate::model::{Stop, Vehicle};

/// A vehicle's route: its stops in order, with when it arrives at each.
#[derive(Clone, Debug)]
pub struct Route {
    pub vehicle: Vehicle,
    pub stops: Vec<Stop>,
    /// Seconds after midnight it arrives at each stop.
    pub arrivals: Vec<u32>,
    /// Seconds after midnight it leaves each stop.
    pub departures: Vec<u32>,
}

impl Route {
    /// The first stop it leaves at or after `t`, where something could go
    /// in after it.
    pub fn stops_after(&self, t: u32) -> Option<usize> {
        self.departures.iter().position(|d| *d >= t).map(|i| i + 1)
    }

    /// When it next leaves a stop after `t`: `t` itself when it never does.
    pub fn next_departure(&self, t: u32) -> u32 {
        self.departures.iter().copied().find(|d| *d > t).unwrap_or(t)
    }

    /// Moves every stop from `i` on later by `seconds`.
    pub fn delay_from(&mut self, i: usize, seconds: u32) {
        for t in self.arrivals.iter_mut().skip(i).chain(self.departures.iter_mut().skip(i)) {
            *t += seconds;
        }
    }
}
