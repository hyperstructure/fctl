//! Runner pools (docs/architecture.md#runner-pools): the runners an
//! enrollment token enrolled, which carry its labels and follow its
//! placement. Projects choose a pool by its labels, and whatever scales a
//! pool reads what it needs now: its runners, its slots, the sessions that
//! wait that it could run, and how many runners would run them all
//! ([`demand`]).

use crate::image::{HostImages, ImageWant};
use crate::schedule::{self, Slot, Wait};
use crate::size::Want;
use crate::validate::Result;
use fctl_proto::ApiError;
use fctl_proto::api::{
    Placement, PoolDemand, PoolInfo, PoolRunners, PoolShape, PoolSlots, PoolWaiting,
};

/// The most labels a pool or a project names.
pub const MAX_LABELS: usize = 16;
/// The longest label.
pub const MAX_LABEL_CHARS: usize = 40;
/// The longest a project's sessions wait for a runner with its labels
/// before any runner takes them, when it says ([`check_fallback`]): a week.
pub const MAX_FALLBACK_MINUTES: u32 = 7 * 24 * 60;

/// Checks the labels of a pool or a project (`field`), and tidies them:
/// lowercase, trimmed, each once, in the order given. A label is letters,
/// digits, `-`, `_`, `.` and `:`, starting with a letter or a digit, as
/// `host.toml`'s labels compare with them as written.
pub fn check_labels(field: &str, labels: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for label in labels {
        let label = label.trim().to_lowercase();
        if label.is_empty() {
            continue;
        }
        let ok = label.chars().count() <= MAX_LABEL_CHARS
            && label
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric())
            && label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'));
        if !ok {
            return Err(ApiError::validation(
                field,
                format!(
                    "{label:?} isn't a label: use letters, digits, -, _, . and :, up to {MAX_LABEL_CHARS} of them, starting with a letter or a digit"
                ),
            ));
        }
        if !out.contains(&label) {
            out.push(label);
        }
    }
    if out.len() > MAX_LABELS {
        return Err(ApiError::validation(
            field,
            format!("Name at most {MAX_LABELS} labels"),
        ));
    }
    Ok(out)
}

/// Checks how long a project's sessions wait for a runner with its labels
/// before any runner takes them: a minute to a week, or for ever (`None`).
pub fn check_fallback(field: &str, minutes: Option<u32>) -> Result<Option<u32>> {
    match minutes {
        None => Ok(None),
        Some(m) if (1..=MAX_FALLBACK_MINUTES).contains(&m) => Ok(Some(m)),
        Some(_) => Err(ApiError::validation(
            field,
            "Let any runner take a session after a minute to a week of waiting, or never",
        )),
    }
}

/// The placement of a new pool that doesn't say: packed when its token
/// removes the runners that go offline (`remove_offline_after_minutes`),
/// as machines that autoscaling makes and destroys are, so that some of
/// them empty out and can go; spread otherwise, as runners always were.
pub fn default_placement(remove_offline_after_minutes: Option<u32>) -> Placement {
    if remove_offline_after_minutes.is_some() {
        Placement::Packed
    } else {
        Placement::Spread
    }
}

/// What a placement does, in a sentence.
pub fn placement_words(p: Placement) -> &'static str {
    match p {
        Placement::Spread => "Spread: each session goes to the least busy runner",
        Placement::Packed => "Packed: fills one runner before the next, so that idle ones can go",
    }
}

/// `the label gpu`, `the labels gpu and large`, `the labels aws, gpu and
/// large`.
pub fn labels_words(labels: &[String]) -> String {
    match labels {
        [] => "no labels".into(),
        [one] => format!("the label {one}"),
        [init @ .., last] => format!("the labels {} and {last}", init.join(", ")),
    }
}

/// A pool's labels and its runners' own (`host.toml`'s), each once: what
/// a project's labels are matched with.
pub fn all_labels(pool: &[String], host: &[String]) -> Vec<String> {
    let mut all = pool.to_vec();
    for l in host {
        if !all.contains(l) {
            all.push(l.clone());
        }
    }
    all
}

// ───────────────────────────── why a session waits ─────────────────────────────

/// A pool as a session that waits for its labels names it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Named<'a> {
    pub name: &'a str,
    /// Its runners that serve sessions or wait to connect: not those that
    /// wait for approval.
    pub runners: u32,
    /// Of those, the ones that never connected yet: on their way up.
    pub starting: u32,
    pub max_runners: Option<u32>,
}

/// What a session whose project asks for labels says while it waits, when
/// a pool has them (`pool`): which pool it waits for, and why, where the
/// pool explains `wait`; `None` to say what `wait` says.
pub fn waiting_for(pool: &Named<'_>, wait: &Wait) -> Option<String> {
    let name = pool.name;
    match wait {
        Wait::Labels { .. } | Wait::Workspace | Wait::NoRunner => Some(match pool.starting {
            0 if pool.runners == 0 => format!("Waiting for {name}: it has no runners yet"),
            0 => format!("Waiting for {name}: none of its runners is online"),
            1 => format!("Waiting for {name}: a runner is starting"),
            n => format!("Waiting for {name}: {n} runners are starting"),
        }),
        Wait::Busy => Some(match pool.max_runners {
            Some(max) if pool.runners >= max => format!(
                "Waiting for {name}: its runners are busy, and it is at its limit of {max} {}",
                if max == 1 { "runner" } else { "runners" }
            ),
            _ => format!("Waiting for {name}: its runners are busy"),
        }),
        _ => None,
    }
}

/// Of the pools that have every one of `labels`, by their own and their
/// runners' (`all_labels`), the one a session that asks for them names
/// and counts towards first: the one with the fewest other labels, then
/// the oldest. Each pool comes with its labels and when its token was
/// made; returns its index.
pub fn first_with<'a>(
    labels: &[String],
    pools: impl Iterator<Item = (&'a [String], i64)>,
) -> Option<usize> {
    pools
        .enumerate()
        .filter(|(_, (have, _))| labels.iter().all(|l| have.contains(l)))
        .min_by_key(|(i, (have, made))| (have.len(), *made, *i))
        .map(|(i, _)| i)
}

// ───────────────────────────── demand ─────────────────────────────

/// One of a pool's runners, as demand counts it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Member {
    /// Its host is connected now.
    pub connected: bool,
    /// It waits for approval.
    pub pending: bool,
    pub draining: bool,
    /// It takes sessions: enabled, and its driver is ready.
    pub ready: bool,
    /// It connected at some point: one that never did is starting.
    pub seen: bool,
    pub capacity: u32,
    /// The sessions assigned to it.
    pub active: u32,
}

/// A pool, as demand counts it.
#[derive(Clone, Debug)]
pub struct Pool {
    pub info: PoolInfo,
    /// When its token was made: of two pools that could run a session,
    /// the older counts it, other things equal.
    pub created_at: i64,
    /// What its runners announced: the latest connected runner's, else the
    /// one the control plane kept.
    pub shape: Option<PoolShape>,
    pub runners: Vec<Member>,
}

/// A session that waits for a runner, as demand counts it.
#[derive(Clone, Copy, Debug)]
pub struct Queued<'a> {
    pub want: Want,
    pub image: &'a ImageWant,
    /// The labels its project asks for (none once it waited past its
    /// project's fallback).
    pub labels: &'a [String],
    pub review: bool,
    /// Whom it runs as.
    pub person: Option<&'a str>,
    /// It runs only on its person's runners (`schedule::only_theirs`).
    pub only_theirs: bool,
    /// Its Claude token is kept on its person's runners: only those that
    /// keep it run it.
    pub on_runners: bool,
    /// Since when it waits.
    pub since: i64,
    /// Why it waits now ([`Wait::kind`]): `home` ones wait for the runner
    /// that has their sandbox and count nowhere until they would start
    /// elsewhere.
    pub kind: &'a str,
}

impl Pool {
    /// Whether a runner of this pool, as it announced itself, would run
    /// `q`, a session of its workspace, its free slots aside. A pool none
    /// of whose runners has connected yet runs any session with its labels
    /// that it runs for, so that it starts its first machine, which then
    /// says what it gives.
    fn runs(&self, q: &Queued<'_>) -> bool {
        let owner = self.info.owner.as_ref().map(|u| u.id.as_str());
        let serves = schedule::runs_for(owner, owner.is_some(), q.person, q.only_theirs)
            && (!q.on_runners || self.shape.as_ref().is_some_and(|s| s.keeps_token));
        let Some(shape) = &self.shape else {
            return serves && q.labels.iter().all(|l| self.info.labels.contains(l));
        };
        let labels = all_labels(&self.info.labels, &shape.labels);
        let slot = Slot {
            id: self.info.id.as_str(),
            name: &self.info.name,
            online: true,
            enabled: true,
            serves,
            capacity: shape.capacity,
            active: 0,
            labels: &labels,
            sandbox: shape.sandbox,
            draining: false,
            low_disk: false,
            images: HostImages {
                own: shape.image.as_deref(),
                allowed: shape.allowed_images.as_deref(),
                builds: shape.allowed_builds.as_deref(),
            },
            reviews: shape.reviews,
            unready: None,
            packed: self.info.placement == Placement::Packed,
        };
        schedule::could_run(&slot, q.labels, q.want, q.image, q.review, false)
    }

    /// Whether it may take another session: a free slot, or, while its
    /// token still enrolls runners, room for another below its
    /// `max_runners`. A closed pool (revoked or expired) can't grow.
    fn has_room(&self) -> bool {
        let enrolled = u32::try_from(self.runners.len()).unwrap_or(u32::MAX);
        let may_grow = !self.info.closed && self.info.max_runners.is_none_or(|max| enrolled < max);
        may_grow || self.slots().free > 0
    }

    fn slots(&self) -> PoolSlots {
        let mut slots = PoolSlots::default();
        for r in self.runners.iter().filter(|r| r.connected && !r.pending) {
            slots.busy += r.active;
            if !r.draining && r.ready {
                slots.capacity += r.capacity;
                slots.free += r.capacity.saturating_sub(r.active);
            }
        }
        slots
    }

    fn counts(&self) -> PoolRunners {
        let mut n = PoolRunners::default();
        for r in &self.runners {
            match () {
                () if r.pending => n.pending += 1,
                () if r.connected && r.draining => n.draining += 1,
                () if r.connected && r.ready => {
                    n.online += 1;
                    if r.active > 0 {
                        n.busy += 1;
                    }
                }
                // Connected, but its driver isn't ready or it is disabled:
                // it runs nothing now, as if it were away.
                () if r.connected || r.seen => n.offline += 1,
                () => n.starting += 1,
            }
        }
        n
    }
}

/// What each of a workspace's pools needs now, from what its runners do
/// and the sessions of the workspace that wait (`queued`), at `now`; with
/// every session that waits, and how many of those no pool could run.
///
/// A session counts towards one pool: of those that could run it, one
/// whose token still enrolls before a revoked or expired one, one that may
/// still grow before one at its limit, one whose runners said what they
/// give before one whose didn't yet, then the one with the fewest labels
/// it doesn't ask for, then the oldest. Sessions that wait
/// for the runner that has their sandbox (`home`), or for their last run
/// to stop there (`stopping`), count nowhere.
///
/// A pool's `wanted` is the runners that would run every session its
/// runners run and every one that counts towards it, at the capacity its
/// runners announce, and at least the runners that run a session now, so
/// that scaling it down to that never takes a busy one; within its
/// `max_runners`, and for a closed pool the runners it has. A pool whose runners never said what they give wants one
/// runner while sessions count towards it.
pub fn demand(
    pools: Vec<Pool>,
    queued: &[Queued<'_>],
    now: i64,
) -> (Vec<PoolDemand>, PoolWaiting, u32) {
    let mut waiting: Vec<PoolWaiting> = vec![PoolWaiting::default(); pools.len()];
    let mut all = PoolWaiting::default();
    let mut unpooled = 0;
    for q in queued {
        add(&mut all, q);
        if matches!(q.kind, "home" | "stopping") {
            continue;
        }
        let first = pools
            .iter()
            .enumerate()
            .filter(|(_, p)| p.runs(q))
            .min_by_key(|(i, p)| {
                let extra = p.info.labels.len() + p.shape.as_ref().map_or(0, |s| s.labels.len());
                (
                    p.info.closed,
                    !p.has_room(),
                    p.shape.is_none(),
                    extra,
                    p.created_at,
                    *i,
                )
            })
            .map(|(i, _)| i);
        match first.and_then(|i| waiting.get_mut(i)) {
            Some(w) => add(w, q),
            None => unpooled += 1,
        }
    }
    let out = pools
        .into_iter()
        .zip(waiting)
        .map(|(pool, waiting)| {
            let runners = pool.counts();
            let slots = pool.slots();
            let wanted = wanted(&pool, slots.busy, runners.busy, waiting.sessions);
            PoolDemand {
                pool: pool.info,
                runners,
                slots,
                waiting,
                wanted,
                shape: pool.shape,
                at: now,
            }
        })
        .collect();
    (out, all, unpooled)
}

/// Counts `q` in `w`.
fn add(w: &mut PoolWaiting, q: &Queued<'_>) {
    w.sessions += 1;
    w.since = Some(w.since.map_or(q.since, |s| s.min(q.since)));
    *w.reasons.entry(q.kind.to_owned()).or_default() += 1;
}

/// How many runners `pool` wants: `running` sessions on its runners, of
/// which `busy` runners run at least one, and `waiting` that count towards
/// it.
fn wanted(pool: &Pool, running: u32, busy: u32, waiting: u32) -> u32 {
    let want = match &pool.shape {
        Some(shape) => (running + waiting)
            .div_ceil(shape.capacity.max(1))
            .max(busy),
        None => u32::from(waiting > 0).max(busy),
    };
    // A closed pool enrolls no more: no more than it has.
    let most = if pool.info.closed {
        Some(u32::try_from(pool.runners.len()).unwrap_or(u32::MAX))
    } else {
        pool.info.max_runners
    };
    most.map_or(want, |max| want.min(max))
}

// ───────────────────────────── Prometheus ─────────────────────────────

/// Pools' demand in Prometheus's text format, for self-hosted control
/// planes (`GET /api/v1/metrics`): each workspace by its slug, each pool
/// by its name and its enrollment token's id. The same numbers as
/// `fctl runners demand`.
pub fn prometheus(workspaces: &[(&str, &fctl_proto::api::WorkspaceDemand)]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let mut family = |name: &str, help: &str, rows: &mut dyn Iterator<Item = (String, u64)>| {
        let _ = writeln!(out, "# HELP {name} {help}");
        let _ = writeln!(out, "# TYPE {name} gauge");
        for (labels, value) in rows {
            let _ = writeln!(out, "{name}{{{labels}}} {value}");
        }
    };
    let pools = || {
        workspaces.iter().flat_map(|(slug, d)| {
            d.pools.iter().map(move |p| {
                (
                    format!(
                        "workspace=\"{}\",pool=\"{}\",pool_id=\"{}\"",
                        escape(slug),
                        escape(&p.pool.name),
                        escape(p.pool.id.as_str())
                    ),
                    p,
                )
            })
        })
    };
    family(
        "fctl_pool_runners",
        "A pool's runners, by what they do now.",
        &mut pools().flat_map(|(l, p)| {
            let r = p.runners;
            [
                ("online", r.online),
                ("draining", r.draining),
                ("pending", r.pending),
                ("starting", r.starting),
                ("offline", r.offline),
            ]
            .into_iter()
            .map(move |(state, n)| (format!("{l},state=\"{state}\""), u64::from(n)))
        }),
    );
    family(
        "fctl_pool_busy_runners",
        "A pool's online runners that run a session.",
        &mut pools().map(|(l, p)| (l, u64::from(p.runners.busy))),
    );
    family(
        "fctl_pool_slots",
        "Session slots on a pool's runners that take sessions: all of them, busy and free.",
        &mut pools().flat_map(|(l, p)| {
            let s = p.slots;
            [("capacity", s.capacity), ("busy", s.busy), ("free", s.free)]
                .into_iter()
                .map(move |(state, n)| (format!("{l},state=\"{state}\""), u64::from(n)))
        }),
    );
    family(
        "fctl_pool_waiting_sessions",
        "Sessions that wait for a runner and count towards a pool.",
        &mut pools().map(|(l, p)| (l, u64::from(p.waiting.sessions))),
    );
    family(
        "fctl_pool_oldest_wait_seconds",
        "How long the session that has waited longest for a pool has waited.",
        &mut pools().map(|(l, p)| {
            let waited = p
                .waiting
                .since
                .map_or(0, |since| (p.at - since).max(0) / 1000);
            (l, u64::try_from(waited).unwrap_or(0))
        }),
    );
    family(
        "fctl_pool_wanted_runners",
        "How many runners would run every session a pool runs and every one that counts towards it.",
        &mut pools().map(|(l, p)| (l, u64::from(p.wanted))),
    );
    family(
        "fctl_waiting_sessions",
        "A workspace's sessions that wait for a runner, by why.",
        &mut workspaces.iter().flat_map(|(slug, d)| {
            d.waiting.reasons.iter().map(move |(reason, n)| {
                (
                    format!(
                        "workspace=\"{}\",reason=\"{}\"",
                        escape(slug),
                        escape(reason)
                    ),
                    u64::from(*n),
                )
            })
        }),
    );
    family(
        "fctl_unpooled_sessions",
        "A workspace's sessions that wait for a runner and that no pool could run.",
        &mut workspaces.iter().map(|(slug, d)| {
            (
                format!("workspace=\"{}\"", escape(slug)),
                u64::from(d.unpooled),
            )
        }),
    );
    out
}

/// A label value as Prometheus reads it: backslashes, quotes and line
/// breaks escaped.
fn escape(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::EnrollmentTokenId;
    use fctl_proto::runner::{SandboxLimits, SandboxSize};

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn labels_are_tidied_and_checked() -> Result<()> {
        assert_eq!(
            check_labels("labels", &s(&[" GPU ", "large", "gpu", "", "us-east-1"]))?,
            s(&["gpu", "large", "us-east-1"])
        );
        assert_eq!(
            check_labels("labels", &s(&["zone:eu", "v1.2_x"]))?,
            s(&["zone:eu", "v1.2_x"])
        );
        for bad in ["-gpu", "has space", "ünïcode", "a/b", &"x".repeat(41)] {
            assert!(check_labels("labels", &s(&[bad])).is_err(), "{bad}");
        }
        let many: Vec<String> = (0..17).map(|i| format!("l{i}")).collect();
        assert!(check_labels("labels", &many).is_err());
        assert_eq!(check_fallback("f", None)?, None);
        assert_eq!(check_fallback("f", Some(10))?, Some(10));
        assert!(check_fallback("f", Some(0)).is_err());
        assert!(check_fallback("f", Some(MAX_FALLBACK_MINUTES + 1)).is_err());
        Ok(())
    }

    #[test]
    fn pools_that_come_and_go_pack_by_default() {
        assert_eq!(default_placement(Some(5)), Placement::Packed);
        assert_eq!(default_placement(None), Placement::Spread);
        assert_eq!(labels_words(&s(&["gpu"])), "the label gpu");
        assert_eq!(
            labels_words(&s(&["aws", "gpu", "large"])),
            "the labels aws, gpu and large"
        );
    }

    #[test]
    fn a_session_waits_for_the_pool_its_labels_name() {
        let pool = Named {
            name: "AWS large",
            runners: 0,
            starting: 0,
            max_runners: Some(20),
        };
        let none_online = Wait::Labels {
            labels: s(&["gpu"]),
        };
        assert_eq!(
            waiting_for(&pool, &none_online).as_deref(),
            Some("Waiting for AWS large: it has no runners yet")
        );
        let starting = Named {
            runners: 2,
            starting: 1,
            ..pool
        };
        assert_eq!(
            waiting_for(&starting, &none_online).as_deref(),
            Some("Waiting for AWS large: a runner is starting")
        );
        let away = Named { runners: 2, ..pool };
        assert_eq!(
            waiting_for(&away, &Wait::Workspace).as_deref(),
            Some("Waiting for AWS large: none of its runners is online")
        );
        assert_eq!(
            waiting_for(&away, &Wait::Busy).as_deref(),
            Some("Waiting for AWS large: its runners are busy")
        );
        let full = Named {
            runners: 20,
            ..pool
        };
        assert_eq!(
            waiting_for(&full, &Wait::Busy).as_deref(),
            Some(
                "Waiting for AWS large: its runners are busy, and it is at its limit of 20 runners"
            )
        );
        assert_eq!(waiting_for(&full, &Wait::Draining), None);
        // Of the pools with the labels, the one with fewest others, then the oldest.
        let pools = [
            (s(&["aws", "gpu", "large"]), 1),
            (s(&["gcp", "gpu"]), 3),
            (s(&["aws", "gpu"]), 2),
            (s(&["cpu"]), 0),
        ];
        let iter = || pools.iter().map(|(l, at)| (l.as_slice(), *at));
        assert_eq!(first_with(&s(&["gpu"]), iter()), Some(2));
        assert_eq!(first_with(&s(&["large"]), iter()), Some(0));
        assert_eq!(first_with(&s(&["tpu"]), iter()), None);
    }

    fn shape(capacity: u32, max_memory_mb: u32) -> PoolShape {
        PoolShape {
            capacity,
            cpus: 16,
            memory_mb: 65_536,
            sandbox: Some(SandboxLimits {
                default: SandboxSize {
                    cpus: 4,
                    memory_mb: 8192,
                },
                max: SandboxSize {
                    cpus: 16,
                    memory_mb: max_memory_mb,
                },
            }),
            image: None,
            allowed_images: None,
            allowed_builds: None,
            driver: "docker".into(),
            version: "0.38.0".into(),
            labels: vec![],
            reviews: true,
            keeps_token: false,
            seen_at: 1,
        }
    }

    fn pool(
        id: &str,
        labels: &[&str],
        max: Option<u32>,
        shape: Option<PoolShape>,
        runners: Vec<Member>,
    ) -> Pool {
        Pool {
            info: PoolInfo {
                id: EnrollmentTokenId(id.into()),
                name: id.into(),
                labels: s(labels),
                placement: Placement::Packed,
                max_runners: max,
                owner: None,
                closed: false,
            },
            created_at: 0,
            shape,
            runners,
        }
    }

    fn up(active: u32) -> Member {
        Member {
            connected: true,
            ready: true,
            seen: true,
            capacity: 4,
            active,
            ..Member::default()
        }
    }

    fn queued<'a>(labels: &'a [String], since: i64, kind: &'a str) -> Queued<'a> {
        Queued {
            want: Want::default(),
            image: &ImageWant::Default,
            labels,
            review: false,
            person: Some("usr_ada"),
            only_theirs: false,
            on_runners: false,
            since,
            kind,
        }
    }

    #[test]
    fn a_pool_wants_the_runners_that_run_what_runs_and_waits() {
        let gpu = s(&["gpu"]);
        let none: Vec<String> = vec![];
        let pools = vec![
            pool(
                "large",
                &["aws", "large"],
                Some(5),
                Some(shape(4, 65_536)),
                vec![
                    up(4),
                    up(1),
                    Member {
                        draining: true,
                        ..up(1)
                    },
                    Member {
                        pending: true,
                        ..Member::default()
                    },
                    Member::default(),
                    Member {
                        seen: true,
                        ..Member::default()
                    },
                ],
            ),
            pool("gpu", &["aws", "gpu"], None, None, vec![]),
        ];
        // Only large has the label, and it gives too little memory.
        let large_label = s(&["large"]);
        let big = Queued {
            want: Want {
                cpus: None,
                memory_mb: Some(131_072),
            },
            ..queued(&large_label, 50, "too_big")
        };
        let queue = [
            queued(&none, 100, "busy"),
            queued(&none, 90, "busy"),
            queued(&gpu, 80, "labels"),
            queued(&none, 70, "home"),
            big,
        ];
        let (pools, all, unpooled) = demand(pools, &queue, 1000);
        let [large, gpu_pool] = pools.as_slice() else {
            panic!("two pools");
        };
        assert_eq!(
            large.runners,
            PoolRunners {
                online: 2,
                busy: 2,
                draining: 1,
                pending: 1,
                starting: 1,
                offline: 1,
            }
        );
        assert_eq!(
            large.slots,
            PoolSlots {
                capacity: 8,
                busy: 6,
                free: 3,
            }
        );
        assert_eq!(large.waiting.sessions, 2);
        assert_eq!(large.waiting.since, Some(90));
        assert_eq!(large.waiting.reasons.get("busy"), Some(&2));
        // (6 running + 2 waiting) / 4 per runner.
        assert_eq!(large.wanted, 2);
        // A pool none of whose runners connected yet wants one, to learn
        // what they give.
        assert_eq!(gpu_pool.waiting.sessions, 1);
        assert_eq!(gpu_pool.wanted, 1);
        // Every session that waits, and the one too big for any pool.
        assert_eq!(all.sessions, 5);
        assert_eq!(all.since, Some(50));
        assert_eq!(all.reasons.get("home"), Some(&1));
        assert_eq!(unpooled, 1);
    }

    #[test]
    fn a_pool_never_wants_fewer_runners_than_run_sessions_nor_more_than_it_may_have() {
        let none: Vec<String> = vec![];
        // Three runners with a session each would fit on one.
        let spread = pool(
            "p",
            &[],
            Some(10),
            Some(shape(4, 65_536)),
            vec![up(1), up(1), up(1), up(0)],
        );
        let (out, _, _) = demand(vec![spread], &[], 0);
        assert_eq!(out.first().map(|p| p.wanted), Some(3));
        // Within its limit.
        let capped = pool("p", &[], Some(2), Some(shape(1, 65_536)), vec![]);
        let queue: Vec<Queued> = (0..5).map(|i| queued(&none, i, "no_runner")).collect();
        let (out, _, _) = demand(vec![capped], &queue, 0);
        assert_eq!(
            out.first().map(|p| (p.waiting.sessions, p.wanted)),
            Some((5, 2))
        );
    }

    #[test]
    fn a_session_counts_towards_one_pool_that_could_run_it() {
        let none: Vec<String> = vec![];
        let large = s(&["large"]);
        let general = pool("general", &["aws"], None, Some(shape(4, 16_384)), vec![]);
        let big = pool(
            "big",
            &["aws", "large"],
            None,
            Some(shape(4, 65_536)),
            vec![],
        );
        let full = pool(
            "full",
            &[],
            Some(1),
            Some(shape(1, 65_536)),
            vec![Member {
                capacity: 1,
                ..up(1)
            }],
        );
        let wants_memory = Queued {
            want: Want {
                cpus: None,
                memory_mb: Some(32_768),
            },
            ..queued(&none, 0, "too_big")
        };
        let queue = [
            // Any runner: the pool with the fewest labels that has room.
            queued(&none, 0, "busy"),
            // Only big has the label.
            queued(&large, 0, "labels"),
            // Only big gives the memory.
            wants_memory,
        ];
        let (out, _, unpooled) = demand(vec![full, big, general], &queue, 0);
        let counted: Vec<(&str, u32)> = out
            .iter()
            .map(|p| (p.pool.name.as_str(), p.waiting.sessions))
            .collect();
        assert_eq!(counted, [("full", 0), ("big", 2), ("general", 1)]);
        assert_eq!(unpooled, 0);
    }

    #[test]
    fn a_revoked_pool_leaves_its_sessions_to_the_one_that_replaced_it() {
        let none: Vec<String> = vec![];
        // The old token, revoked, still has a runner and a shape; its
        // replacement, with the same labels, has neither yet.
        let mut old = pool("old", &["gpu"], None, Some(shape(4, 65_536)), vec![up(4)]);
        old.info.closed = true;
        old.created_at = 1;
        let mut new = pool("new", &["gpu"], None, None, vec![]);
        new.created_at = 2;
        let gpu = s(&["gpu"]);
        let queue = [queued(&gpu, 0, "busy"), queued(&none, 0, "busy")];
        let (out, _, _) = demand(vec![old, new], &queue, 0);
        let counted: Vec<(&str, u32, u32)> = out
            .iter()
            .map(|p| (p.pool.name.as_str(), p.waiting.sessions, p.wanted))
            .collect();
        // The new pool starts a runner for them; the old one wants no more
        // than the runner it has.
        assert_eq!(counted, [("old", 0, 1), ("new", 2, 1)]);
    }

    #[test]
    fn a_members_pool_counts_only_their_sessions() {
        let none: Vec<String> = vec![];
        let mut mine = pool("mine", &[], None, Some(shape(2, 65_536)), vec![]);
        mine.info.owner = Some(fctl_proto::session::UserRef {
            id: fctl_proto::UserId("usr_ada".into()),
            name: "Ada".into(),
            avatar_url: None,
        });
        let theirs = Queued {
            person: Some("usr_grace"),
            ..queued(&none, 0, "busy")
        };
        let token_kept = Queued {
            on_runners: true,
            ..queued(&none, 0, "busy")
        };
        let (out, _, unpooled) = demand(
            vec![mine.clone()],
            &[queued(&none, 0, "busy"), theirs, token_kept],
            0,
        );
        assert_eq!(out.first().map(|p| p.waiting.sessions), Some(1));
        assert_eq!(unpooled, 2);
        // Once its runners keep Ada's Claude token, theirs too.
        if let Some(shape) = mine.shape.as_mut() {
            shape.keeps_token = true;
        }
        let (out, _, _) = demand(vec![mine], &[token_kept], 0);
        assert_eq!(out.first().map(|p| p.waiting.sessions), Some(1));
    }

    #[test]
    fn pools_read_as_prometheus_metrics() {
        let none: Vec<String> = vec![];
        let pools = vec![pool(
            "enr_large",
            &["aws"],
            Some(5),
            Some(shape(4, 65_536)),
            vec![up(2), Member::default()],
        )];
        let queue = [queued(&none, 40_000, "busy"), queued(&none, 70_000, "home")];
        let (pools, waiting, unpooled) = demand(pools, &queue, 100_000);
        let mut d = fctl_proto::api::WorkspaceDemand {
            pools,
            waiting,
            unpooled,
            at: 100_000,
        };
        if let Some(p) = d.pools.first_mut() {
            p.pool.name = "AWS \"large\"".into();
        }
        let text = prometheus(&[("acme", &d)]);
        let l = r#"workspace="acme",pool="AWS \"large\"",pool_id="enr_large""#;
        for line in [
            "# TYPE fctl_pool_runners gauge".to_owned(),
            format!(r#"fctl_pool_runners{{{l},state="online"}} 1"#),
            format!(r#"fctl_pool_runners{{{l},state="starting"}} 1"#),
            format!("fctl_pool_busy_runners{{{l}}} 1"),
            format!(r#"fctl_pool_slots{{{l},state="free"}} 2"#),
            format!("fctl_pool_waiting_sessions{{{l}}} 1"),
            format!("fctl_pool_oldest_wait_seconds{{{l}}} 60"),
            format!("fctl_pool_wanted_runners{{{l}}} 1"),
            r#"fctl_waiting_sessions{workspace="acme",reason="home"} 1"#.to_owned(),
            r#"fctl_unpooled_sessions{workspace="acme"} 0"#.to_owned(),
        ] {
            assert!(text.lines().any(|t| t == line), "{line}\n{text}");
        }
    }
}
