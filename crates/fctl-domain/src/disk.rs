//! Free disk on hosts: the floor a host keeps where its sandboxes are.
//!
//! Below it, the host removes stopped sandboxes, those stopped longest
//! first, until it is above it again, and takes no new sessions meanwhile:
//! only resumes of the sessions whose sandboxes it has, which need no new
//! one. The control plane places sessions by the same rule.

use crate::size;

/// Whether a host with `free_mb` of disk free is below the floor it keeps,
/// `min_free_mb`. Unknown free space, or no floor, is never low.
pub fn low(free_mb: Option<u64>, min_free_mb: Option<u64>) -> bool {
    matches!((free_mb, min_free_mb), (Some(free), Some(min)) if free < min)
}

/// `12.3 GiB of disk free, below the 20 GiB it keeps free`.
pub fn below(free_mb: u64, min_free_mb: u64) -> String {
    format!(
        "{} of disk free, below the {} it keeps free",
        size::memory(free_mb),
        size::memory(min_free_mb)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_measured_disk_below_a_floor_is_low() {
        let floor = Some(20 * 1024);
        assert!(low(Some(12_595), floor));
        assert!(!low(Some(20 * 1024), floor));
        assert!(!low(Some(500_000), floor));
        // Unknown free space, and hosts that keep no floor or are older.
        assert!(!low(None, floor));
        assert!(!low(Some(0), None));
    }

    #[test]
    fn says_how_far_below_the_floor_it_is() {
        assert_eq!(
            below(12_595, 20 * 1024),
            "12.3 GiB of disk free, below the 20 GiB it keeps free"
        );
        assert_eq!(
            below(800, 1024),
            "800 MiB of disk free, below the 1 GiB it keeps free"
        );
    }
}
