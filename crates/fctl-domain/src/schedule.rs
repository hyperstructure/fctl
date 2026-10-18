//! Choosing a runner for a session.

use crate::size::{self, Want};
use fctl_proto::runner::{SandboxLimits, SandboxSize};

/// A runner as the scheduler sees it.
#[derive(Clone, Debug)]
pub struct Slot<'a> {
    pub id: &'a str,
    pub online: bool,
    pub enabled: bool,
    pub capacity: u32,
    pub active: u32,
    pub labels: &'a [String],
    /// What it gives each sandbox. `None` (older hosts) takes any size.
    pub sandbox: Option<SandboxLimits>,
}

/// Why no runner takes a session now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Wait {
    /// No runner with the labels it needs is online.
    NoRunner,
    /// The runners that could take it run as many sessions as they may.
    Busy,
    /// No online runner gives a sandbox this many CPUs or this much memory.
    TooBig {
        /// The part of its size no runner gives (`None`: some runner gives it).
        cpus: Option<u32>,
        memory_mb: Option<u32>,
        /// The most CPUs, and the most memory, any one runner gives.
        most: SandboxSize,
    },
    /// Runners give its CPUs and runners give its memory, but none gives both.
    NoneGivesBoth { cpus: u32, memory_mb: u32 },
}

impl Wait {
    /// What the queued session says.
    pub fn reason(&self) -> String {
        match self {
            Self::NoRunner | Self::Busy => "Waiting for a runner".into(),
            Self::TooBig {
                cpus,
                memory_mb,
                most,
            } => {
                let asked = parts(*cpus, *memory_mb, " of memory");
                let most = parts(
                    cpus.map(|_| most.cpus),
                    memory_mb.map(|_| most.memory_mb),
                    "",
                );
                if asked.is_empty() {
                    "No runner can give this session its sandbox size".into()
                } else {
                    format!("No runner can give this session {asked} (the most is {most})")
                }
            }
            Self::NoneGivesBoth { cpus, memory_mb } => format!(
                "No runner can give this session both {} and {} of memory",
                size::cpus(*cpus),
                size::memory((*memory_mb).into())
            ),
        }
    }
}

/// `16 CPUs and 32 GiB of memory`, or either part.
fn parts(cpus: Option<u32>, memory_mb: Option<u32>, memory_suffix: &str) -> String {
    let cpus = cpus.map(size::cpus);
    let memory = memory_mb.map(|m| format!("{}{memory_suffix}", size::memory(m.into())));
    cpus.into_iter()
        .chain(memory)
        .collect::<Vec<_>>()
        .join(" and ")
}

/// The least loaded online runner with a free slot that has every required
/// label and gives a sandbox the size the session wants. Ties go to the
/// runner listed first.
pub fn pick<'a>(
    slots: &'a [Slot<'a>],
    required_labels: &[String],
    want: Want,
) -> Result<&'a Slot<'a>, Wait> {
    let online: Vec<&Slot> = slots
        .iter()
        .filter(|s| s.online && s.enabled)
        .filter(|s| required_labels.iter().all(|l| s.labels.contains(l)))
        .collect();
    if online.is_empty() {
        return Err(Wait::NoRunner);
    }
    let fitting: Vec<&Slot> = online
        .iter()
        .copied()
        .filter(|s| s.sandbox.is_none_or(|l| want.fits(&l)))
        .collect();
    if fitting.is_empty() {
        return Err(too_big(&online, want));
    }
    fitting
        .into_iter()
        .filter(|s| s.active < s.capacity)
        .min_by(|a, b| {
            // Compare active/capacity without floats: a.active * b.cap vs b.active * a.cap.
            let la = a.active as u64 * b.capacity.max(1) as u64;
            let lb = b.active as u64 * a.capacity.max(1) as u64;
            la.cmp(&lb)
        })
        .ok_or(Wait::Busy)
}

/// Why no online runner fits `want` (all of them report limits).
fn too_big(online: &[&Slot], want: Want) -> Wait {
    let limits = || online.iter().filter_map(|s| s.sandbox);
    let most = SandboxSize {
        cpus: limits().map(|l| l.max.cpus).max().unwrap_or(0),
        memory_mb: limits().map(|l| l.max.memory_mb).max().unwrap_or(0),
    };
    let cpus = want.cpus.filter(|&c| c > most.cpus);
    let memory_mb = want.memory_mb.filter(|&m| m > most.memory_mb);
    match (cpus, memory_mb, want.cpus, want.memory_mb) {
        (None, None, Some(cpus), Some(memory_mb)) => Wait::NoneGivesBoth { cpus, memory_mb },
        _ => Wait::TooBig {
            cpus,
            memory_mb,
            most,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot<'a>(id: &'a str, capacity: u32, active: u32, labels: &'a [String]) -> Slot<'a> {
        Slot {
            id,
            online: true,
            enabled: true,
            capacity,
            active,
            labels,
            sandbox: None,
        }
    }

    fn limits(max_cpus: u32, max_memory_mb: u32) -> Option<SandboxLimits> {
        Some(SandboxLimits {
            default: SandboxSize {
                cpus: 4.min(max_cpus),
                memory_mb: 8192.min(max_memory_mb),
            },
            max: SandboxSize {
                cpus: max_cpus,
                memory_mb: max_memory_mb,
            },
        })
    }

    fn memory(mb: u32) -> Want {
        Want {
            cpus: None,
            memory_mb: Some(mb),
        }
    }

    #[test]
    fn picks_least_loaded() {
        let none: Vec<String> = vec![];
        let gpu = vec!["gpu".to_string()];
        let slots = vec![
            slot("a", 4, 2, &none),
            slot("b", 8, 2, &gpu),
            Slot {
                online: false,
                ..slot("c", 8, 0, &none)
            },
            slot("d", 1, 1, &none),
        ];
        let any = Want::default();
        assert_eq!(pick(&slots, &[], any).map(|s| s.id), Ok("b"));
        assert_eq!(pick(&slots, &gpu, any).map(|s| s.id), Ok("b"));
        assert_eq!(
            pick(&slots[2..3], &[], any).map(|s| s.id),
            Err(Wait::NoRunner)
        );
        assert_eq!(pick(&slots[3..], &[], any).map(|s| s.id), Err(Wait::Busy));
    }

    #[test]
    fn picks_a_runner_that_gives_the_size() {
        let none: Vec<String> = vec![];
        let slots = vec![
            Slot {
                sandbox: limits(10, 15_990),
                ..slot("docker-vm", 4, 0, &none)
            },
            Slot {
                sandbox: limits(16, 65_536),
                ..slot("studio", 4, 3, &none)
            },
        ];
        // The idle runner first, while the session fits there.
        assert_eq!(
            pick(&slots, &[], memory(8192)).map(|s| s.id),
            Ok("docker-vm")
        );
        assert_eq!(
            pick(&slots, &[], memory(16_384)).map(|s| s.id),
            Ok("studio")
        );
        let many = Want {
            cpus: Some(12),
            memory_mb: None,
        };
        assert_eq!(pick(&slots, &[], many).map(|s| s.id), Ok("studio"));
        // The only runner that fits is full: it waits for that one.
        let full = [
            slots[0].clone(),
            Slot {
                active: 4,
                ..slots[1].clone()
            },
        ];
        assert_eq!(
            pick(&full, &[], memory(16_384)).map(|s| s.id),
            Err(Wait::Busy)
        );
    }

    #[test]
    fn older_hosts_take_any_size() {
        let none: Vec<String> = vec![];
        let slots = [slot("old", 4, 0, &none)];
        assert_eq!(pick(&slots, &[], memory(262_144)).map(|s| s.id), Ok("old"));
    }

    #[test]
    fn says_what_no_runner_gives() {
        let none: Vec<String> = vec![];
        let slots = [
            Slot {
                sandbox: limits(10, 15_990),
                ..slot("a", 4, 0, &none)
            },
            Slot {
                sandbox: limits(4, 65_536),
                ..slot("b", 4, 0, &none)
            },
        ];
        let reason = |want: Want| pick(&slots, &[], want).err().map(|w| w.reason());
        assert_eq!(
            reason(memory(131_072)).as_deref(),
            Some("No runner can give this session 128 GiB of memory (the most is 64 GiB)")
        );
        assert_eq!(
            reason(Want {
                cpus: Some(16),
                memory_mb: None
            })
            .as_deref(),
            Some("No runner can give this session 16 CPUs (the most is 10 CPUs)")
        );
        assert_eq!(
            reason(Want {
                cpus: Some(16),
                memory_mb: Some(131_072)
            })
            .as_deref(),
            Some(
                "No runner can give this session 16 CPUs and 128 GiB of memory (the most is 10 CPUs and 64 GiB)"
            )
        );
        // a has the CPUs, b has the memory.
        assert_eq!(
            reason(Want {
                cpus: Some(8),
                memory_mb: Some(32_768)
            })
            .as_deref(),
            Some("No runner can give this session both 8 CPUs and 32 GiB of memory")
        );
    }
}
