//! The search's state: routes under construction, and the moves that
//! improve them.

use crate::insert::insert_visit;
use crate::model::{Problem, Stop};
use crate::route::Route;
use crate::solve::Solution;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The threads local search runs its moves on, made once for the process.
static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();

pub struct Search<'p> {
    problem: &'p Problem,
    routes: Vec<Route>,
    unassigned: Vec<String>,
    rng: u64,
}

impl<'p> Search<'p> {
    pub fn new(problem: &'p Problem, seed: u64) -> Self {
        let routes = problem
            .vehicles
            .iter()
            .map(|v| Route {
                vehicle: v.clone(),
                stops: vec![Stop::Depot(v.depot), Stop::Depot(v.depot)],
                arrivals: vec![v.shift.start, v.shift.start],
                departures: vec![v.shift.start, v.shift.start],
            })
            .collect();
        Self { problem, routes, unassigned: Vec::new(), rng: seed | 1 }
    }

    /// Cheapest insertion, visits with the tightest windows first.
    pub fn construct(&mut self) {
        let mut visits = self.problem.visits.clone();
        visits.sort_by_key(|v| v.window.map_or(u32::MAX, |w| w.end - w.start));
        for v in &visits {
            let placed = self.routes.iter_mut().any(|r| insert_visit(r, v, |r, i| super::cost::added(r, v, i)).is_ok());
            if !placed {
                self.unassigned.push(v.id.clone());
            }
        }
    }

    /// 2-opt, relocate and exchange moves until `time` is up.
    pub fn improve_for(&mut self, time: Duration) {
        let until = Instant::now() + time;
        let pool = POOL.get_or_init(|| rayon::ThreadPoolBuilder::new().build().expect("a thread pool"));
        while Instant::now() < until {
            if !pool.install(|| super::moves::improve_once(&mut self.routes, &mut self.rng)) {
                break;
            }
        }
    }

    pub fn into_solution(self) -> Solution {
        let cost = self.routes.iter().map(super::cost::route).sum();
        Solution { routes: self.routes, unassigned: self.unassigned, cost }
    }
}
