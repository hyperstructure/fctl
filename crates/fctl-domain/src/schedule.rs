//! Choosing a runner for a session.

/// A runner as the scheduler sees it.
#[derive(Clone, Debug)]
pub struct Slot<'a> {
    pub id: &'a str,
    pub online: bool,
    pub enabled: bool,
    pub capacity: u32,
    pub active: u32,
    pub labels: &'a [String],
}

/// The least loaded online runner with a free slot that has every required
/// label. Ties go to the runner listed first.
pub fn pick<'a>(slots: &'a [Slot<'a>], required_labels: &[String]) -> Option<&'a Slot<'a>> {
    slots
        .iter()
        .filter(|s| s.online && s.enabled && s.active < s.capacity)
        .filter(|s| required_labels.iter().all(|l| s.labels.contains(l)))
        .min_by(|a, b| {
            // Compare active/capacity without floats: a.active * b.cap vs b.active * a.cap.
            let la = a.active as u64 * b.capacity.max(1) as u64;
            let lb = b.active as u64 * a.capacity.max(1) as u64;
            la.cmp(&lb)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_least_loaded() {
        let none: Vec<String> = vec![];
        let gpu = vec!["gpu".to_string()];
        let slots = vec![
            Slot {
                id: "a",
                online: true,
                enabled: true,
                capacity: 4,
                active: 2,
                labels: &none,
            },
            Slot {
                id: "b",
                online: true,
                enabled: true,
                capacity: 8,
                active: 2,
                labels: &gpu,
            },
            Slot {
                id: "c",
                online: false,
                enabled: true,
                capacity: 8,
                active: 0,
                labels: &none,
            },
            Slot {
                id: "d",
                online: true,
                enabled: true,
                capacity: 1,
                active: 1,
                labels: &none,
            },
        ];
        assert_eq!(pick(&slots, &[]).map(|s| s.id), Some("b"));
        assert_eq!(pick(&slots, &gpu).map(|s| s.id), Some("b"));
        assert_eq!(pick(&slots[2..], &[]).map(|s| s.id), None);
    }
}
