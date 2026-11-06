//! Free disk on hosts: the floor a host keeps where its sandboxes are, and
//! what happens as a host runs short of room.
//!
//! Below the floor, the host removes stopped sandboxes, those stopped
//! longest first, until it is above it again, and takes no new sessions
//! meanwhile: only resumes of the sessions whose sandboxes it has, which need
//! no new one. The control plane places sessions by the same rule.
//!
//! Below [`OUT_MB`] (or the floor, when that is lower) the host is nearly
//! out of disk: what its sandboxes write is about to fail. Its sessions hear
//! of both from the control plane ([`heard`]), and a host pauses the
//! sandboxes that are VMs while it is nearly out, since what they write
//! would be lost there without a word.

use crate::size;
use fctl_proto::event::{Notice, NoticeLevel};
use fctl_proto::runner::{DiskRoom, RunnerDisk};
use serde::{Deserialize, Serialize};

/// Below this much free disk, or the floor when that is lower, a host is
/// nearly out of disk. A fixed amount rather than a share of the floor: it
/// is what the sessions running there have left to write, which doesn't
/// grow with the disk, and a host that keeps a small floor still gets a
/// warning before its disk is full.
pub const OUT_MB: u64 = 2 * 1024;

/// How long a runner must have had more room before its sessions are told
/// that it does: a host that makes room only just above its floor, and
/// falls below it again as its sessions write, tells them once.
pub const BETTER_AFTER_MS: i64 = 10 * 60_000;

/// How short of room a host with `free_mb` of disk free is, against the
/// floor it keeps, `min_free_mb`. Unknown free space, or no floor, is never
/// short.
pub fn room(free_mb: Option<u64>, min_free_mb: Option<u64>) -> Option<DiskRoom> {
    let (free, min) = (free_mb?, min_free_mb?);
    if free < OUT_MB.min(min) {
        Some(DiskRoom::Out)
    } else if free < min {
        Some(DiskRoom::Low)
    } else {
        None
    }
}

/// Whether a host with `free_mb` of disk free is below the floor it keeps,
/// `min_free_mb`. Unknown free space, or no floor, is never low.
pub fn low(free_mb: Option<u64>, min_free_mb: Option<u64>) -> bool {
    room(free_mb, min_free_mb).is_some()
}

/// `12.3 GiB of disk free, below the 20 GiB it keeps free`.
pub fn below(free_mb: u64, min_free_mb: u64) -> String {
    format!(
        "{} of disk free, below the {} it keeps free",
        size::memory(free_mb),
        size::memory(min_free_mb)
    )
}

/// What a runner's sessions were told about its disk, which the control
/// plane keeps with the runner.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Told {
    /// How short of room it was at its last heartbeat: none with room.
    #[serde(default)]
    pub room: Option<DiskRoom>,
    /// Since when it has been so.
    #[serde(default)]
    pub since: i64,
    /// What its sessions' transcripts said last.
    #[serde(default)]
    pub noticed: Option<DiskRoom>,
    /// The free MiB that its sessions' pages show.
    #[serde(default)]
    pub shown_mb: Option<u64>,
}

/// What to tell a runner's sessions after one of its heartbeats.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tell {
    /// For their transcripts.
    pub notice: Option<Notice>,
    /// Whether what their pages show of its disk changed.
    pub changed: bool,
    /// What their pages show of its disk now: nothing while it has room.
    pub disk: Option<RunnerDisk>,
}

/// A heartbeat of runner `runner` said it has `free_mb` of disk free, and
/// its host keeps `min_free_mb`: what its sessions hear, and what to keep.
///
/// They hear at once when it has less room than they were told (below the
/// floor, then nearly out), and once it has had more for
/// [`BETTER_AFTER_MS`] when it has more, so that a disk that goes up and
/// down around a line isn't told at each crossing. What their pages show
/// follows each heartbeat: when how short of room it is changes, and when
/// its free space moves by a tenth. A heartbeat that couldn't measure the
/// disk changes nothing.
pub fn heard(
    told: &Told,
    runner: &str,
    free_mb: Option<u64>,
    min_free_mb: Option<u64>,
    now: i64,
) -> (Told, Tell) {
    let Some(free) = free_mb else {
        return (told.clone(), Tell::default());
    };
    let room = room(free_mb, min_free_mb);
    let mut next = told.clone();
    if room != told.room {
        next.room = room;
        next.since = now;
    }
    let worse = room > told.noticed;
    let better = room < told.noticed && now - next.since >= BETTER_AFTER_MS;
    let notice = (worse || better)
        .then(|| notice(runner, told.noticed, room, free, min_free_mb.unwrap_or(0)));
    if notice.is_some() {
        next.noticed = room;
    }
    let changed = room != told.room || (room.is_some() && moved(told.shown_mb, free));
    if changed {
        next.shown_mb = room.map(|_| free);
    }
    let disk = shown(&next, free_mb, min_free_mb);
    (
        next,
        Tell {
            notice,
            changed,
            disk,
        },
    )
}

/// Whether free space shown as `shown` has moved enough by now to show
/// again: by a tenth, and at least 64 MiB.
fn moved(shown: Option<u64>, free: u64) -> bool {
    shown.is_none_or(|was| {
        let by = was.abs_diff(free);
        by >= 64 && by.saturating_mul(10) >= was
    })
}

/// What a session's page shows of its runner's disk, as `told` has it.
pub fn shown(told: &Told, free_mb: Option<u64>, min_free_mb: Option<u64>) -> Option<RunnerDisk> {
    Some(RunnerDisk {
        room: told.room?,
        free_mb: free_mb?,
        min_free_mb: min_free_mb?,
        since: told.since,
    })
}

/// What the transcript of a session that starts on a runner short of room
/// says.
pub fn joined(
    told: &Told,
    runner: &str,
    free_mb: Option<u64>,
    min_free_mb: Option<u64>,
) -> Option<Notice> {
    let disk = shown(told, free_mb, min_free_mb)?;
    Some(notice(
        runner,
        None,
        Some(disk.room),
        disk.free_mb,
        disk.min_free_mb,
    ))
}

/// The notice that runner `runner` went from being `from` short of room to
/// being `to`: `runner_low_on_disk`, `runner_out_of_disk` or
/// `runner_disk_ok`.
pub fn notice(
    runner: &str,
    from: Option<DiskRoom>,
    to: Option<DiskRoom>,
    free_mb: u64,
    min_free_mb: u64,
) -> Notice {
    let (free, floor) = (size::memory(free_mb), size::memory(min_free_mb));
    match (from, to) {
        (_, Some(DiskRoom::Out)) => Notice::new(
            NoticeLevel::Error,
            "runner_out_of_disk",
            format!(
                "Runner {runner} is nearly out of disk: {free} free. The agent's writes are about to fail, until its host has removed stopped sandboxes or space is freed on it."
            ),
        ),
        (Some(DiskRoom::Out), Some(DiskRoom::Low)) => Notice::new(
            NoticeLevel::Warn,
            "runner_low_on_disk",
            format!(
                "Runner {runner} has more room on its disk again: {free} free, still below the {floor} its host keeps free, so no new sessions start on it yet."
            ),
        ),
        (_, Some(DiskRoom::Low)) => Notice::new(
            NoticeLevel::Warn,
            "runner_low_on_disk",
            format!(
                "Runner {runner} is low on disk: {free} free, below the {floor} its host keeps free. Until there is room, no new sessions start on it, and its host removes stopped sandboxes early. If its disk fills up, the agent's writes fail."
            ),
        ),
        (_, None) => Notice::new(
            NoticeLevel::Info,
            "runner_disk_ok",
            format!("Runner {runner} has room on its disk again: {free} free."),
        ),
    }
}

/// Whether an error, or one it comes from, is a disk with no room left:
/// the system's `ENOSPC`, or a program that said "No space left on device".
pub fn out_of_space(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut next = Some(err);
    while let Some(e) = next {
        if e.downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::StorageFull)
            || says_out_of_space(&e.to_string())
        {
            return true;
        }
        next = e.source();
    }
    false
}

/// Whether what a program wrote says that its disk had no room left.
pub fn says_out_of_space(text: &str) -> bool {
    text.to_ascii_lowercase()
        .contains("no space left on device")
        || text.contains("ENOSPC")
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024;
    const FLOOR: Option<u64> = Some(20 * GIB);
    const MIN: i64 = 60_000;

    #[test]
    fn only_a_measured_disk_below_a_floor_is_low() {
        assert!(low(Some(12_595), FLOOR));
        assert!(!low(Some(20 * 1024), FLOOR));
        assert!(!low(Some(500_000), FLOOR));
        // Unknown free space, and hosts that keep no floor or are older.
        assert!(!low(None, FLOOR));
        assert!(!low(Some(0), None));
    }

    #[test]
    fn nearly_out_is_below_two_gib_or_the_floor_if_lower() {
        assert_eq!(room(Some(25 * GIB), FLOOR), None);
        assert_eq!(room(Some(12 * GIB), FLOOR), Some(DiskRoom::Low));
        assert_eq!(room(Some(2 * GIB), FLOOR), Some(DiskRoom::Low));
        assert_eq!(room(Some(2 * GIB - 1), FLOOR), Some(DiskRoom::Out));
        assert_eq!(room(Some(0), FLOOR), Some(DiskRoom::Out));
        // A host that keeps 1 GiB is nearly out below it.
        assert_eq!(room(Some(1500), Some(GIB)), None);
        assert_eq!(room(Some(900), Some(GIB)), Some(DiskRoom::Out));
        // No floor, no measurement: never short.
        assert_eq!(room(Some(0), None), None);
        assert_eq!(room(None, FLOOR), None);
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

    /// The notice codes a run of heartbeats gives, one per heartbeat a
    /// minute apart.
    fn codes(free: &[u64]) -> Vec<Option<String>> {
        let mut told = Told::default();
        free.iter()
            .enumerate()
            .map(|(i, f)| {
                let (next, tell) = heard(&told, "now", Some(*f), FLOOR, i as i64 * MIN);
                told = next;
                tell.notice.map(|n| n.code)
            })
            .collect()
    }

    #[test]
    fn sessions_hear_once_when_their_runner_goes_below_its_floor() {
        let (told, tell) = heard(&Told::default(), "now", Some(12_595), FLOOR, 1_000);
        let notice = tell.notice.unwrap_or_else(|| panic!("a notice"));
        assert_eq!(notice.level, NoticeLevel::Warn);
        assert_eq!(notice.code, "runner_low_on_disk");
        assert_eq!(
            notice.message,
            "Runner now is low on disk: 12.3 GiB free, below the 20 GiB its host keeps free. Until there is room, no new sessions start on it, and its host removes stopped sandboxes early. If its disk fills up, the agent's writes fail."
        );
        assert!(tell.changed);
        assert_eq!(
            tell.disk,
            Some(RunnerDisk {
                room: DiskRoom::Low,
                free_mb: 12_595,
                min_free_mb: 20 * GIB,
                since: 1_000
            })
        );
        // The next heartbeats say about the same: nothing to tell.
        let (told, tell) = heard(&told, "now", Some(12_400), FLOOR, 16_000);
        assert_eq!((tell.notice, tell.changed), (None, false));
        assert_eq!(
            tell.disk.map(|d| (d.free_mb, d.since)),
            Some((12_400, 1_000))
        );
        // A tenth less: their pages show it, their transcripts don't.
        let (_, tell) = heard(&told, "now", Some(11_000), FLOOR, 31_000);
        assert_eq!((tell.notice, tell.changed), (None, true));
    }

    #[test]
    fn nearly_out_is_an_error_at_once() {
        let (told, _) = heard(&Told::default(), "now", Some(5 * GIB), FLOOR, 0);
        let (told, tell) = heard(&told, "now", Some(800), FLOOR, MIN);
        let notice = tell.notice.unwrap_or_else(|| panic!("a notice"));
        assert_eq!(notice.level, NoticeLevel::Error);
        assert_eq!(notice.code, "runner_out_of_disk");
        assert_eq!(
            notice.message,
            "Runner now is nearly out of disk: 800 MiB free. The agent's writes are about to fail, until its host has removed stopped sandboxes or space is freed on it."
        );
        assert_eq!(
            tell.disk.map(|d| (d.room, d.since)),
            Some((DiskRoom::Out, MIN))
        );
        assert_eq!(told.noticed, Some(DiskRoom::Out));
    }

    #[test]
    fn more_room_is_told_once_it_has_lasted_ten_minutes() {
        // Down, then back above the floor: the page's line goes at once,
        // the transcript says so ten minutes later.
        let mut free = vec![25 * GIB, 12 * GIB, 21 * GIB];
        free.extend([22 * GIB; 10]);
        let codes = codes(&free);
        assert_eq!(codes[1].as_deref(), Some("runner_low_on_disk"));
        assert!(codes[2..12].iter().all(Option::is_none), "{codes:?}");
        assert_eq!(codes[12].as_deref(), Some("runner_disk_ok"));

        let (told, _) = heard(&Told::default(), "now", Some(12 * GIB), FLOOR, 0);
        let (told, tell) = heard(&told, "now", Some(21 * GIB), FLOOR, MIN);
        assert_eq!((tell.changed, tell.disk), (true, None));
        let (_, tell) = heard(&told, "now", Some(21 * GIB), FLOOR, 11 * MIN);
        let notice = tell.notice.unwrap_or_else(|| panic!("a notice"));
        assert_eq!(notice.level, NoticeLevel::Info);
        assert_eq!(
            notice.message,
            "Runner now has room on its disk again: 21 GiB free."
        );
    }

    #[test]
    fn a_disk_that_goes_up_and_down_around_its_floor_is_told_once() {
        // The host makes room just above its floor, and its sessions fill
        // it again, every few minutes.
        let free = [
            25 * GIB,
            19 * GIB,
            21 * GIB,
            19 * GIB,
            20 * GIB + 512,
            19 * GIB,
            21 * GIB,
        ];
        let told: Vec<_> = codes(&free).into_iter().flatten().collect();
        assert_eq!(told, ["runner_low_on_disk"]);
    }

    #[test]
    fn nearly_out_and_back_says_what_room_it_has_now() {
        let mut free = vec![12 * GIB, GIB, 3 * GIB];
        free.extend([3 * GIB; 10]);
        let codes: Vec<_> = codes(&free).into_iter().flatten().collect();
        assert_eq!(
            codes,
            [
                "runner_low_on_disk",
                "runner_out_of_disk",
                "runner_low_on_disk"
            ]
        );
        let back = notice(
            "now",
            Some(DiskRoom::Out),
            Some(DiskRoom::Low),
            3 * GIB,
            20 * GIB,
        );
        assert_eq!(
            back.message,
            "Runner now has more room on its disk again: 3 GiB free, still below the 20 GiB its host keeps free, so no new sessions start on it yet."
        );
    }

    #[test]
    fn a_disk_that_wasnt_measured_or_has_no_floor_tells_nothing() {
        let (told, _) = heard(&Told::default(), "now", Some(12 * GIB), FLOOR, 0);
        let (same, tell) = heard(&told, "now", None, FLOOR, MIN);
        assert_eq!(same, told);
        assert_eq!(tell, Tell::default());
        let (_, tell) = heard(&Told::default(), "now", Some(0), None, 0);
        assert_eq!((tell.notice, tell.changed, tell.disk), (None, false, None));
    }

    #[test]
    fn sessions_that_start_on_a_runner_short_of_room_hear_how_short() {
        assert_eq!(joined(&Told::default(), "now", Some(30 * GIB), FLOOR), None);
        let (told, _) = heard(&Told::default(), "now", Some(GIB), FLOOR, 0);
        let notice = joined(&told, "now", Some(GIB), FLOOR);
        assert_eq!(
            notice.map(|n| n.code).as_deref(),
            Some("runner_out_of_disk")
        );
    }

    #[test]
    fn what_is_kept_reads_back() -> serde_json::Result<()> {
        let (told, _) = heard(&Told::default(), "now", Some(GIB), FLOOR, 5);
        let json = serde_json::to_string(&told)?;
        assert_eq!(serde_json::from_str::<Told>(&json)?, told);
        assert_eq!(serde_json::from_str::<Told>("{}")?, Told::default());
        Ok(())
    }

    #[derive(Debug)]
    struct Saving(std::io::Error);

    impl std::fmt::Display for Saving {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("saving the state")
        }
    }

    impl std::error::Error for Saving {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn knows_a_full_disk_from_the_system_and_from_what_programs_say() {
        let full = std::io::Error::from_raw_os_error(28);
        assert!(out_of_space(&full));
        assert!(out_of_space(&Saving(full)));
        let said =
            std::io::Error::other("docker: write /var/lib/docker/tmp: no space left on device");
        assert!(out_of_space(&Saving(said)));
        assert!(!out_of_space(&Saving(std::io::Error::other(
            "permission denied"
        ))));
        assert!(!out_of_space(&std::io::Error::from_raw_os_error(13)));
        assert!(says_out_of_space(
            "Error: ENOSPC: no space left on device, write"
        ));
        assert!(says_out_of_space(
            "fatal: write error: No space left on device"
        ));
        assert!(!says_out_of_space("Disk space: plenty"));
    }
}
