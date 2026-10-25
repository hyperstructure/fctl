//! Properties every solution has, on random problems.

use milkfloat::solve;
use proptest::prelude::*;
use std::time::Duration;

mod gen;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Every visit is on exactly one route, or unassigned.
    #[test]
    fn every_visit_is_placed_once(problem in gen::problem(1..4, 0..40)) {
        let s = solve(&problem, Duration::from_millis(50), 1).unwrap();
        let mut seen: Vec<String> = s.routes.iter().flat_map(gen::visit_ids).chain(s.unassigned.clone()).collect();
        seen.sort();
        let mut want: Vec<String> = problem.visits.iter().map(|v| v.id.clone()).collect();
        want.sort();
        prop_assert_eq!(seen, want);
    }

    /// No route arrives anywhere outside a hard window, or runs past its shift.
    #[test]
    fn hard_windows_and_shifts_hold(problem in gen::problem(1..4, 0..40)) {
        let s = solve(&problem, Duration::from_millis(50), 1).unwrap();
        for r in &s.routes {
            prop_assert!(gen::keeps_hard_windows(r));
            prop_assert!(r.departures.last().copied().unwrap_or(0) <= r.vehicle.shift.end);
        }
    }
}
