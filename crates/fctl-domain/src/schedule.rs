//! Choosing a runner for a session.

use crate::image::{HostImages, ImageWant};
use crate::size::{self, Want};
use fctl_proto::runner::{SandboxLimits, SandboxSize};

/// A runner as the scheduler sees it.
#[derive(Clone, Debug)]
pub struct Slot<'a> {
    pub id: &'a str,
    pub online: bool,
    pub enabled: bool,
    /// It runs the sessions of the session's workspace.
    pub serves: bool,
    pub capacity: u32,
    pub active: u32,
    pub labels: &'a [String],
    /// What it gives each sandbox. `None` (older hosts) takes any size.
    pub sandbox: Option<SandboxLimits>,
    /// It drains: no new session starts on it, but the sessions whose
    /// sandboxes it has still resume there.
    pub draining: bool,
    /// Its disk is below the free space it keeps ([`crate::disk`]): as
    /// while it drains, only the sessions whose sandboxes it has start on
    /// it, since they need no new one.
    pub low_disk: bool,
    /// The images it runs and builds.
    pub images: HostImages<'a>,
    /// Its host runs review agents' sessions (`fctl_proto::runner::FEATURE_REVIEWS`).
    pub reviews: bool,
}

/// The runner that keeps a session's sandbox from an earlier run: there,
/// Claude continues the same conversation, with the work it left in the
/// sandbox.
#[derive(Clone, Copy, Debug)]
pub struct Home<'a> {
    pub id: &'a str,
    pub name: &'a str,
    /// Whether the session waits for it while it runs as many sessions as
    /// it may, rather than starting elsewhere in a new sandbox.
    pub wait: bool,
}

/// Why no runner takes a session now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Wait {
    /// No runner with the labels it needs is online.
    NoRunner,
    /// Runners are online, but none of them runs its workspace's sessions.
    Workspace,
    /// The runners that could take it run as many sessions as they may.
    Busy,
    /// The runner that keeps its sandbox runs as many sessions as it may.
    Home { name: String },
    /// Every online runner that could take it drains.
    Draining,
    /// Every online runner that could take it is low on disk, or drains
    /// (`draining`: some do).
    LowDisk { draining: bool },
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
    /// No online runner runs the image its project chose.
    Image { image: String },
    /// No online runner builds images from its project's repository.
    Build { repo: String },
    /// It is a review agent's session, and no online runner of its
    /// workspace runs a version of fctl that has review agents.
    Reviews,
}

impl Wait {
    /// What the queued session says.
    pub fn reason(&self) -> String {
        match self {
            Self::NoRunner | Self::Busy => "Waiting for a runner".into(),
            Self::Workspace => {
                "Waiting for a runner: none of this workspace's runners is online".into()
            }
            Self::Home { name } => format!("Waiting for {name}, which has this session's sandbox"),
            Self::Draining => "Waiting for a runner: the ones online are draining".into(),
            Self::LowDisk { draining: false } => {
                "Waiting for a runner: the ones online are low on disk".into()
            }
            Self::LowDisk { draining: true } => {
                "Waiting for a runner: the ones online are draining or low on disk".into()
            }
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
            Self::Image { image } => format!("No runner allows this project's image, {image}"),
            Self::Build { repo } => format!("No runner builds this project's image from {repo}"),
            Self::Reviews => "Waiting for a runner that runs review agents: the ones online run an older fctl. Update them with `fctl self update`.".into(),
        }
    }

    /// Why no runner takes a session that wants `image`.
    fn image(image: &ImageWant) -> Self {
        match image {
            ImageWant::Build(repo) => Self::Build {
                repo: repo.full_name(),
            },
            ImageWant::Image(image) => Self::Image {
                image: image.clone(),
            },
            // Every runner runs its own image.
            ImageWant::Default => Self::NoRunner,
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
/// label, runs or builds the image the session wants, gives a sandbox the
/// size it wants and, for a review agent's session (`review`), runs review
/// agents. Ties go to the runner listed first.
///
/// A session's `home` comes first when it is online and could take it, also
/// while it drains or is low on disk: with a free slot it gets the session,
/// and without one the session waits for it if `home.wait` says so. Other
/// runners that drain or are low on disk take no sessions.
pub fn pick<'a>(
    slots: &'a [Slot<'a>],
    required_labels: &[String],
    want: Want,
    image: &ImageWant,
    home: Option<Home<'_>>,
    review: bool,
) -> Result<&'a Slot<'a>, Wait> {
    let online: Vec<&Slot> = slots
        .iter()
        .filter(|s| s.online && s.enabled)
        .filter(|s| required_labels.iter().all(|l| s.labels.contains(l)))
        .collect();
    if online.is_empty() {
        return Err(Wait::NoRunner);
    }
    let online: Vec<&Slot> = online.into_iter().filter(|s| s.serves).collect();
    if online.is_empty() {
        return Err(Wait::Workspace);
    }
    let online: Vec<&Slot> = online
        .into_iter()
        .filter(|s| !review || s.reviews)
        .collect();
    if online.is_empty() {
        return Err(Wait::Reviews);
    }
    let online: Vec<&Slot> = online
        .into_iter()
        .filter(|s| s.images.takes(image))
        .collect();
    if online.is_empty() {
        return Err(Wait::image(image));
    }
    if let Some(home) = home
        && let Some(slot) = online
            .iter()
            .copied()
            .find(|s| s.id == home.id && s.sandbox.is_none_or(|l| want.fits(&l)))
    {
        if slot.active < slot.capacity {
            return Ok(slot);
        }
        if home.wait {
            return Err(Wait::Home {
                name: home.name.to_owned(),
            });
        }
    }
    let open: Vec<&Slot> = online
        .iter()
        .copied()
        .filter(|s| !s.draining && !s.low_disk)
        .collect();
    if open.is_empty() {
        let draining = online.iter().any(|s| s.draining);
        return Err(if online.iter().all(|s| s.draining) {
            Wait::Draining
        } else {
            Wait::LowDisk { draining }
        });
    }
    let fitting: Vec<&Slot> = open
        .iter()
        .copied()
        .filter(|s| s.sandbox.is_none_or(|l| want.fits(&l)))
        .collect();
    if fitting.is_empty() {
        return Err(too_big(&open, want));
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

/// Whether a runner of a session's workspace runs the session, by whose
/// sessions it runs there and whom the session runs as. `owner` registered
/// the runner, and `personal` says it runs only their sessions in the
/// workspace (they added it for their own). `person` owns the Claude
/// connection the session runs on, and with `own_runners` keeps their
/// sessions in the workspace on runners they registered, as they do too
/// when that connection's token is kept on their runners ([`only_theirs`]).
pub fn runs_for(
    owner: Option<&str>,
    personal: bool,
    person: Option<&str>,
    own_runners: bool,
) -> bool {
    let theirs = person.is_some() && owner == person;
    theirs || !(personal || own_runners)
}

/// Whether a session runs only on its person's runners: they keep their
/// sessions in its workspace on runners they registered (`own_runners`),
/// or the Claude connection it runs on keeps its token on their runners
/// (`on_runners`), which no one else's runner has. Pass it to [`runs_for`]
/// as `own_runners`.
pub fn only_theirs(own_runners: bool, on_runners: bool) -> bool {
    own_runners || on_runners
}

/// Why a session waits whose Claude token its person keeps on their
/// runners, when no online runner of its workspace takes it
/// ([`Wait::Workspace`]): only their runners that keep the token
/// (`fctl_proto::runner::RunnerClaudeToken`) take it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenWait {
    /// None of their runners runs the workspace's sessions.
    NoRunner,
    /// Theirs run them, but none of those is online.
    Offline,
    /// Of theirs online, none keeps the token. `old`: some of them run an
    /// fctl too old to keep one.
    NoToken { old: bool },
}

impl TokenWait {
    /// What the queued session says, naming its person (`name`).
    pub fn reason(&self, name: &str) -> String {
        let name = match name.trim() {
            "" => "its person",
            name => name,
        };
        match self {
            Self::NoRunner => format!(
                "Waiting for one of {name}'s runners: their Claude token is kept on their runners, and none of those runs this workspace's sessions yet. They add one on its Runners tab."
            ),
            Self::Offline => format!(
                "Waiting for one of {name}'s runners: their Claude token is kept on their runners, and none of those is online"
            ),
            Self::NoToken { old: false } => format!(
                "Waiting for one of {name}'s runners that keeps their Claude token: the ones online keep none. `fctl host claude set-token` on one of them keeps it there."
            ),
            Self::NoToken { old: true } => format!(
                "Waiting for one of {name}'s runners that keeps their Claude token: the ones online keep none, and some run an fctl too old to keep one. Update them with `fctl self update`, then run `fctl host claude set-token` on one."
            ),
        }
    }
}

/// Why a session waits when no online runner of its workspace takes it
/// ([`Wait::Workspace`]). `own`: the name of the person it runs as, when
/// they keep their sessions in the workspace on their own runners.
/// `takes`: one of the workspace's runners that is offline now would take
/// it; `any`: the workspace has runners at all; `others_online`: some that
/// run only their owners' sessions are online.
pub fn no_runner_takes(own: Option<&str>, takes: bool, any: bool, others_online: bool) -> String {
    match own.map(str::trim) {
        Some(name) => {
            let name = if name.is_empty() { "its person" } else { name };
            if takes {
                format!(
                    "Waiting for one of {name}'s runners: their sessions in this workspace run only on their own, and none of those is online"
                )
            } else {
                format!(
                    "Waiting for one of {name}'s runners: their sessions in this workspace run only on their own, and none of theirs runs its sessions yet. They add one on its Runners tab."
                )
            }
        }
        None if takes && others_online => "Waiting for a runner: the ones online here run only their owners' sessions, and the others are offline".into(),
        None if takes => Wait::Workspace.reason(),
        None if any => "Waiting for a runner: this workspace's runners run only their owners' sessions. Its admins add one for everyone's on its Runners tab.".into(),
        None => "Waiting for a runner: this workspace has none yet. Its admins add one on its Runners tab.".into(),
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
            serves: true,
            capacity,
            active,
            labels,
            sandbox: None,
            draining: false,
            low_disk: false,
            images: HostImages::default(),
            reviews: false,
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
        assert_eq!(
            pick(&slots, &[], any, &ImageWant::Default, None, false).map(|s| s.id),
            Ok("b")
        );
        assert_eq!(
            pick(&slots, &gpu, any, &ImageWant::Default, None, false).map(|s| s.id),
            Ok("b")
        );
        assert_eq!(
            pick(&slots[2..3], &[], any, &ImageWant::Default, None, false).map(|s| s.id),
            Err(Wait::NoRunner)
        );
        assert_eq!(
            pick(&slots[3..], &[], any, &ImageWant::Default, None, false).map(|s| s.id),
            Err(Wait::Busy)
        );
    }

    #[test]
    fn picks_only_runners_of_the_sessions_workspace() {
        let none: Vec<String> = vec![];
        let any = Want::default();
        let slots = vec![
            Slot {
                serves: false,
                ..slot("theirs", 8, 0, &none)
            },
            slot("ours", 4, 3, &none),
        ];
        // The busier runner, since the idle one serves another workspace.
        assert_eq!(
            pick(&slots, &[], any, &ImageWant::Default, None, false).map(|s| s.id),
            Ok("ours")
        );
        assert_eq!(
            pick(&slots[..1], &[], any, &ImageWant::Default, None, false).map(|s| s.id),
            Err(Wait::Workspace)
        );
        // Even the runner that has the session's sandbox, once it no longer
        // serves the workspace.
        let home = Home {
            id: "theirs",
            name: "theirs",
            wait: true,
        };
        assert_eq!(
            pick(&slots, &[], any, &ImageWant::Default, Some(home), false).map(|s| s.id),
            Ok("ours")
        );
        assert_eq!(
            Wait::Workspace.reason(),
            "Waiting for a runner: none of this workspace's runners is online"
        );
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
            pick(&slots, &[], memory(8192), &ImageWant::Default, None, false).map(|s| s.id),
            Ok("docker-vm")
        );
        assert_eq!(
            pick(
                &slots,
                &[],
                memory(16_384),
                &ImageWant::Default,
                None,
                false
            )
            .map(|s| s.id),
            Ok("studio")
        );
        let many = Want {
            cpus: Some(12),
            memory_mb: None,
        };
        assert_eq!(
            pick(&slots, &[], many, &ImageWant::Default, None, false).map(|s| s.id),
            Ok("studio")
        );
        // The only runner that fits is full: it waits for that one.
        let full = [
            slots[0].clone(),
            Slot {
                active: 4,
                ..slots[1].clone()
            },
        ];
        assert_eq!(
            pick(&full, &[], memory(16_384), &ImageWant::Default, None, false).map(|s| s.id),
            Err(Wait::Busy)
        );
    }

    #[test]
    fn older_hosts_take_any_size() {
        let none: Vec<String> = vec![];
        let slots = [slot("old", 4, 0, &none)];
        assert_eq!(
            pick(
                &slots,
                &[],
                memory(262_144),
                &ImageWant::Default,
                None,
                false
            )
            .map(|s| s.id),
            Ok("old")
        );
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
        let reason = |want: Want| {
            pick(&slots, &[], want, &ImageWant::Default, None, false)
                .err()
                .map(|w| w.reason())
        };
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

    fn home(id: &str, wait: bool) -> Option<Home<'_>> {
        Some(Home { id, name: id, wait })
    }

    #[test]
    fn a_resume_goes_back_to_its_sandbox() {
        let none: Vec<String> = vec![];
        let slots = [slot("idle", 4, 0, &none), slot("studio", 4, 3, &none)];
        let any = Want::default();
        // Busier, but it has the sandbox.
        assert_eq!(
            pick(
                &slots,
                &[],
                any,
                &ImageWant::Default,
                home("studio", true),
                false
            )
            .map(|s| s.id),
            Ok("studio")
        );
        // Full: the session waits for it, or starts elsewhere once it no
        // longer waits.
        let full = [
            slots[0].clone(),
            Slot {
                active: 4,
                ..slots[1].clone()
            },
        ];
        assert_eq!(
            pick(
                &full,
                &[],
                any,
                &ImageWant::Default,
                home("studio", true),
                false
            )
            .map(|s| s.id),
            Err(Wait::Home {
                name: "studio".into()
            })
        );
        assert_eq!(
            pick(
                &full,
                &[],
                any,
                &ImageWant::Default,
                home("studio", false),
                false
            )
            .map(|s| s.id),
            Ok("idle")
        );
        assert_eq!(
            Wait::Home {
                name: "studio".into()
            }
            .reason(),
            "Waiting for studio, which has this session's sandbox"
        );
    }

    #[test]
    fn a_resume_whose_home_cannot_take_it_starts_elsewhere() {
        let none: Vec<String> = vec![];
        let slots = [
            slot("idle", 4, 0, &none),
            Slot {
                online: false,
                ..slot("asleep", 4, 0, &none)
            },
            Slot {
                sandbox: limits(4, 8192),
                active: 4,
                ..slot("small", 4, 4, &none)
            },
        ];
        // Offline: it doesn't wait.
        assert_eq!(
            pick(
                &slots,
                &[],
                Want::default(),
                &ImageWant::Default,
                home("asleep", true),
                false
            )
            .map(|s| s.id),
            Ok("idle")
        );
        // Too small now (the project asks for more): it doesn't wait either.
        assert_eq!(
            pick(
                &slots,
                &[],
                memory(16_384),
                &ImageWant::Default,
                home("small", true),
                false
            )
            .map(|s| s.id),
            Ok("idle")
        );
        // Gone.
        assert_eq!(
            pick(
                &slots,
                &[],
                Want::default(),
                &ImageWant::Default,
                home("removed", true),
                false
            )
            .map(|s| s.id),
            Ok("idle")
        );
    }

    #[test]
    fn a_draining_runner_takes_only_resumes_of_its_own_sessions() {
        let none: Vec<String> = vec![];
        let slots = [
            Slot {
                draining: true,
                ..slot("studio", 4, 0, &none)
            },
            slot("laptop", 1, 0, &none),
        ];
        let any = Want::default();
        // Emptier, but draining: new sessions go elsewhere.
        assert_eq!(
            pick(&slots, &[], any, &ImageWant::Default, None, false).map(|s| s.id),
            Ok("laptop")
        );
        // A resume goes back to its sandbox there.
        assert_eq!(
            pick(
                &slots,
                &[],
                any,
                &ImageWant::Default,
                home("studio", true),
                false
            )
            .map(|s| s.id),
            Ok("studio")
        );
        // The other one is full: a new session waits, and says why when
        // every runner online drains.
        let full = [
            slots[0].clone(),
            Slot {
                active: 1,
                ..slots[1].clone()
            },
        ];
        assert_eq!(
            pick(&full, &[], any, &ImageWant::Default, None, false).map(|s| s.id),
            Err(Wait::Busy)
        );
        let draining = [slots[0].clone()];
        assert_eq!(
            pick(&draining, &[], any, &ImageWant::Default, None, false).map(|s| s.id),
            Err(Wait::Draining)
        );
        assert_eq!(
            Wait::Draining.reason(),
            "Waiting for a runner: the ones online are draining"
        );
    }

    #[test]
    fn a_runner_low_on_disk_takes_only_resumes_of_its_own_sessions() {
        let none: Vec<String> = vec![];
        let slots = [
            Slot {
                low_disk: true,
                ..slot("studio", 4, 0, &none)
            },
            slot("laptop", 1, 0, &none),
        ];
        let any = Want::default();
        let new = |slots: &[Slot]| {
            pick(slots, &[], any, &ImageWant::Default, None, false)
                .map(|s| s.id.to_owned())
                .map_err(|w| w.reason())
        };
        // Emptier, but low on disk: new sessions go elsewhere.
        assert_eq!(new(&slots), Ok("laptop".into()));
        // A resume goes back to its sandbox there, which needs no new one.
        assert_eq!(
            pick(
                &slots,
                &[],
                any,
                &ImageWant::Default,
                home("studio", true),
                false
            )
            .map(|s| s.id),
            Ok("studio")
        );
        // With no other runner, a new session waits and says why.
        assert_eq!(
            new(&slots[..1]),
            Err("Waiting for a runner: the ones online are low on disk".into())
        );
        // One low on disk and one draining.
        let closed = [
            slots[0].clone(),
            Slot {
                draining: true,
                ..slots[1].clone()
            },
        ];
        assert_eq!(
            new(&closed),
            Err("Waiting for a runner: the ones online are draining or low on disk".into())
        );
        // One that drains and is low on disk too counts as draining.
        let both = [Slot {
            draining: true,
            ..slots[0].clone()
        }];
        assert_eq!(
            new(&both),
            Err("Waiting for a runner: the ones online are draining".into())
        );
    }

    #[test]
    fn sessions_go_to_runners_that_allow_their_image() {
        use fctl_proto::session::RepoRef;
        let none: Vec<String> = vec![];
        let acme = vec!["ghcr.io/acme/*".to_owned()];
        let only_own: Vec<String> = vec![];
        let builds = vec!["acme/*".to_owned()];
        let slots = [
            Slot {
                images: HostImages {
                    own: Some("ghcr.io/hyperstructure/sandbox:0.5.0"),
                    allowed: Some(&only_own),
                    builds: None,
                },
                ..slot("strict", 4, 0, &none)
            },
            Slot {
                images: HostImages {
                    own: None,
                    allowed: Some(&acme),
                    builds: Some(&builds),
                },
                ..slot("acme", 4, 3, &none)
            },
        ];
        let any = Want::default();
        let image = |i: &str| ImageWant::Image(i.into());
        // The default image runs everywhere: the emptier runner.
        assert_eq!(
            pick(&slots, &[], any, &ImageWant::Default, None, false).map(|s| s.id),
            Ok("strict")
        );
        // A project's image only where its owner allows it.
        assert_eq!(
            pick(
                &slots,
                &[],
                any,
                &image("ghcr.io/acme/agent:1"),
                None,
                false
            )
            .map(|s| s.id),
            Ok("acme")
        );
        let wait = pick(
            &slots,
            &[],
            any,
            &image("docker.io/evil/miner"),
            None,
            false,
        );
        assert_eq!(
            wait.clone().map(|s| s.id),
            Err(Wait::Image {
                image: "docker.io/evil/miner".into()
            })
        );
        assert_eq!(
            wait.err().map(|w| w.reason()).as_deref(),
            Some("No runner allows this project's image, docker.io/evil/miner")
        );
        // A build only where its owner builds from the repository.
        let app = ImageWant::Build(RepoRef {
            owner: "acme".into(),
            name: "app".into(),
        });
        assert_eq!(
            pick(&slots, &[], any, &app, None, false).map(|s| s.id),
            Ok("acme")
        );
        let other = ImageWant::Build(RepoRef {
            owner: "other".into(),
            name: "app".into(),
        });
        assert_eq!(
            pick(&slots, &[], any, &other, None, false)
                .err()
                .map(|w| w.reason())
                .as_deref(),
            Some("No runner builds this project's image from other/app")
        );
        // A resume doesn't go back to a runner that no longer allows it.
        assert_eq!(
            pick(
                &slots,
                &[],
                any,
                &image("ghcr.io/acme/agent:1"),
                home("strict", true),
                false
            )
            .map(|s| s.id),
            Ok("acme")
        );
        // Older hosts send no list and run any image, but build none.
        let old = [slot("old", 4, 0, &none)];
        assert_eq!(
            pick(&old, &[], any, &image("docker.io/evil/miner"), None, false).map(|s| s.id),
            Ok("old")
        );
        assert_eq!(
            pick(&old, &[], any, &app, None, false).map(|s| s.id),
            Err(Wait::Build {
                repo: "acme/app".into()
            })
        );
    }

    #[test]
    fn personal_runners_and_own_runners_keep_sessions_to_their_people() {
        let (ada, bo) = (Some("usr_ada"), Some("usr_bo"));
        // A runner for everyone's sessions takes anyone's, unless they keep
        // theirs on their own runners.
        assert!(runs_for(ada, false, bo, false));
        assert!(!runs_for(ada, false, bo, true));
        assert!(runs_for(ada, false, ada, true));
        // One for its owner's sessions takes only theirs.
        assert!(runs_for(ada, true, ada, false));
        assert!(runs_for(ada, true, ada, true));
        assert!(!runs_for(ada, true, bo, false));
        // A session that runs as nobody, and a runner nobody owns, belong
        // to no one.
        assert!(runs_for(ada, false, None, false));
        assert!(!runs_for(ada, true, None, false));
        assert!(runs_for(None, false, bo, false));
        assert!(!runs_for(None, false, bo, true));
        assert!(!runs_for(None, true, None, false));
    }

    #[test]
    fn sessions_say_whose_runners_they_wait_for() {
        assert_eq!(
            no_runner_takes(None, true, true, false),
            "Waiting for a runner: none of this workspace's runners is online"
        );
        assert!(no_runner_takes(None, true, true, true).contains("the others are offline"));
        assert!(no_runner_takes(None, false, false, false).contains("has none yet"));
        assert!(no_runner_takes(None, false, true, true).contains("only their owners' sessions"));
        assert_eq!(
            no_runner_takes(Some("Ada Lovelace"), true, true, true),
            "Waiting for one of Ada Lovelace's runners: their sessions in this workspace run only on their own, and none of those is online"
        );
        assert!(
            no_runner_takes(Some("Ada Lovelace"), false, true, true).ends_with(
                "none of theirs runs its sessions yet. They add one on its Runners tab."
            )
        );
        assert!(
            no_runner_takes(Some(" "), true, true, false)
                .starts_with("Waiting for one of its person's runners")
        );
    }

    #[test]
    fn sessions_on_a_token_kept_on_runners_run_only_on_their_persons() {
        let (ada, bo) = (Some("usr_ada"), Some("usr_bo"));
        // Even where their person runs their sessions on anyone's runners.
        let only = only_theirs(false, true);
        assert!(only);
        assert!(!runs_for(bo, false, ada, only));
        assert!(runs_for(ada, false, ada, only));
        assert!(runs_for(ada, true, ada, only));
        assert!(!runs_for(None, false, ada, only));
        assert!(!only_theirs(false, false));
        assert!(only_theirs(true, false));
    }

    #[test]
    fn sessions_on_a_token_kept_on_runners_say_what_they_wait_for() {
        assert_eq!(
            TokenWait::Offline.reason("Ada Lovelace"),
            "Waiting for one of Ada Lovelace's runners: their Claude token is kept on their runners, and none of those is online"
        );
        assert!(
            TokenWait::NoRunner
                .reason("Ada Lovelace")
                .ends_with("none of those runs this workspace's sessions yet. They add one on its Runners tab.")
        );
        assert!(
            TokenWait::NoToken { old: false }
                .reason("Ada Lovelace")
                .contains("`fctl host claude set-token`")
        );
        let old = TokenWait::NoToken { old: true }.reason("Ada Lovelace");
        assert!(old.contains("`fctl self update`"), "{old}");
        assert!(
            TokenWait::Offline
                .reason(" ")
                .starts_with("Waiting for one of its person's runners")
        );
    }

    #[test]
    fn review_sessions_go_to_runners_that_run_review_agents() {
        let none: Vec<String> = vec![];
        let any = Want::default();
        let slots = [
            slot("old", 4, 0, &none),
            Slot {
                reviews: true,
                ..slot("new", 4, 3, &none)
            },
        ];
        // Other sessions go anywhere, the least loaded first.
        assert_eq!(
            pick(&slots, &[], any, &ImageWant::Default, None, false).map(|s| s.id),
            Ok("old")
        );
        assert_eq!(
            pick(&slots, &[], any, &ImageWant::Default, None, true).map(|s| s.id),
            Ok("new")
        );
        let wait = pick(&slots[..1], &[], any, &ImageWant::Default, None, true).map(|s| s.id);
        assert_eq!(wait, Err(Wait::Reviews));
        assert!(Wait::Reviews.reason().contains("fctl self update"));
    }
}
