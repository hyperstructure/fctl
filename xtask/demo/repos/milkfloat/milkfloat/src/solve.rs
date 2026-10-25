//! The search: cheapest insertion to start, then local search (2-opt,
//! relocate, exchange) until the time is up.

use crate::model::Problem;
use crate::route::Route;
use std::time::Duration;

#[derive(Debug)]
pub struct Solution {
    pub routes: Vec<Route>,
    /// Visits no route could take.
    pub unassigned: Vec<String>,
    pub cost: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum SolveError {
    #[error("the problem has no vehicles")]
    NoVehicles,
    #[error("vehicle {0} has a shift that ends before it starts")]
    Shift(String),
}

/// Solves `problem`, searching for `time` with the given seed.
pub fn solve(problem: &Problem, time: Duration, seed: u64) -> Result<Solution, SolveError> {
    if problem.vehicles.is_empty() {
        return Err(SolveError::NoVehicles);
    }
    if let Some(v) = problem.vehicles.iter().find(|v| v.shift.end < v.shift.start) {
        return Err(SolveError::Shift(v.id.clone()));
    }
    let mut search = crate::solve::search::Search::new(problem, seed);
    search.construct();
    search.improve_for(time);
    Ok(search.into_solution())
}

mod search;
