//! Apple's `container`: sandboxes that lost their network, which the host
//! brings back by restarting Apple's `container`.
//!
//! Apple's `container` gives each sandbox's VM its network's gateway as
//! nameserver (`192.168.64.1` on the default network), where macOS's vmnet
//! answers lookups and carries the VM's traffic out. That can stop working
//! for every sandbox at once while the Mac's own network works on: lookups
//! in sandboxes run out of time (`Could not resolve host: github.com`,
//! `EAI_AGAIN`), or their connections go nowhere, after the Mac's network
//! changed or a VPN came or went, or when a network's bridge went with its
//! last VM ([apple/container#1881], [#1986], [#2051]). It doesn't come back
//! by itself, while `container system stop` and `container system start`,
//! which make the network again, bring it back.
//!
//! So before fctl-agent starts in a sandbox, and every [`LOOK_EVERY`] while
//! sessions' sandboxes run, the host asks fctl-agent in the sandbox whether
//! it reaches [`reach::TARGET`] (`fctl-agent reach`). When it doesn't, the
//! host asks this Mac itself, whose own network a restart wouldn't bring
//! back (it remembers a Mac off the network for [`OFFLINE_FOR`]), then a
//! fresh sandbox, twice: a sandbox may keep itself off the network (its
//! `/etc/hosts`, say), while a fresh one runs only the host's own
//! fctl-agent, from a directory that holds nothing else. When fresh
//! sandboxes don't reach it either, the host restarts Apple's `container`,
//! at most once every [`RESTART_EVERY`], in a task that nothing calls off,
//! and starts again the containers that ran: what ran in them is gone, as
//! after the Mac restarts, and their files stay. Sandboxes that a restart
//! doesn't bring back make the driver not ready, saying why: sessions wait
//! for a runner rather than fail, and the host looks again every
//! [`LOOK_EVERY`], with a fresh sandbox, or after [`DOWN_HOLDS`] lets
//! sessions look for it when no fresh sandbox can tell.
//!
//! [apple/container#1881]: https://github.com/apple/container/issues/1881
//! [#1986]: https://github.com/apple/container/issues/1986
//! [#2051]: https://github.com/apple/container/issues/2051

use fctl_domain::reach;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How long a sandbox, or this Mac, may take to reach [`reach::TARGET`]:
/// a lookup that runs out of time takes 5 seconds a nameserver.
pub const REACH_WITHIN: Duration = Duration::from_secs(10);

/// How often the host looks whether sessions' sandboxes reach the network
/// while they run, and whether a network it couldn't bring back is back.
pub const LOOK_EVERY: Duration = Duration::from_secs(60);

/// How long after restarting Apple's `container` the host may restart it
/// again: a restart stops every sandbox on the Mac, and one that didn't
/// bring the network back may not the next time either.
pub const RESTART_EVERY: Duration = Duration::from_secs(30 * 60);

/// How long the host waits before it asks a second fresh sandbox whether it
/// reaches the network, when the first didn't: a network whose bridge has
/// just come may not carry anything for a moment.
pub const CONFIRM_AFTER: Duration = Duration::from_secs(5);

/// How long the host remembers that this Mac itself doesn't reach the
/// network, and doesn't look at its sandboxes' meanwhile.
pub const OFFLINE_FOR: Duration = Duration::from_secs(5 * 60);

/// How long sandboxes that didn't reach the network keep the driver not
/// ready while no fresh sandbox can tell whether they do now (none of the
/// images is here, say): then sessions may start, and look themselves.
pub const DOWN_HOLDS: Duration = Duration::from_secs(10 * 60);

/// How long `container system stop` may take: it stops every container,
/// giving each 5 seconds, and waits 20 more for them.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// Where a fresh sandbox finds the host's fctl-agent, which it runs to say
/// whether it reaches the network.
pub(crate) const REACH_DIR: &str = "/opt/fctl-reach";

/// The names of fresh sandboxes: what the host starts again after a
/// restart leaves them out.
pub(crate) const REACH_PREFIX: &str = "fctl-reach-";

/// How long one address may take to take a connection, of the time there
/// is.
const PER_ADDRESS: Duration = Duration::from_secs(5);

/// What the host knows of its sandboxes' network.
pub(crate) struct Network {
    /// Held (to write) while the host restarts Apple's `container`: what
    /// starts a sandbox, or fctl-agent in one, waits for it (to read).
    pub restarting: tokio::sync::RwLock<()>,
    /// Held while the host looks at its sandboxes' network, and brings it
    /// back: one look at a time.
    pub looking: tokio::sync::Mutex<()>,
    /// Where sandboxes should reach ([`reach::TARGET`]).
    pub target: String,
    state: Mutex<State>,
}

impl Default for Network {
    fn default() -> Self {
        Self {
            restarting: tokio::sync::RwLock::new(()),
            looking: tokio::sync::Mutex::new(()),
            target: reach::TARGET.into(),
            state: Mutex::default(),
        }
    }
}

#[derive(Default)]
struct State {
    /// When the host last restarted Apple's `container` for its sandboxes'
    /// network.
    restarted: Option<Instant>,
    /// When it last looked whether they reach it, by itself.
    looked: Option<Instant>,
    /// Why they don't, which the host couldn't bring back, and since when
    /// it knows: the driver isn't ready meanwhile.
    down: Option<(String, Instant)>,
    /// When this Mac last didn't reach the network itself.
    offline: Option<Instant>,
    /// The containers that ran before a restart of Apple's `container` and
    /// that it hasn't started again yet.
    left: Vec<String>,
    /// The host's fctl-agent and the images the last session that started
    /// went by, for the fresh sandboxes the host starts by itself.
    fresh: Option<(PathBuf, Vec<String>)>,
    /// The sandboxes that didn't reach it while a fresh one did, and when:
    /// something in them keeps them from it, and the host doesn't ask them
    /// again for [`RESTART_EVERY`].
    own_doing: HashMap<String, Instant>,
}

impl Network {
    /// What the host knows of sandboxes that should reach `target`, for
    /// tests.
    #[cfg(test)]
    pub fn reaching(target: &str) -> Self {
        Self {
            target: target.to_owned(),
            ..Self::default()
        }
    }

    /// Why sandboxes don't reach the network, which the host couldn't
    /// bring back; `None` while they do, as far as it knows.
    pub fn down(&self) -> Option<String> {
        self.state().down.as_ref().map(|(why, _)| why.clone())
    }

    /// [`Self::down`], with when the host last found it so.
    pub fn down_since(&self) -> Option<(String, Instant)> {
        self.state().down.clone()
    }

    /// What the host found, now: why sandboxes don't reach the network, or
    /// that they do.
    pub fn set_down(&self, why: Option<String>) {
        self.state().down = why.map(|why| (why, Instant::now()));
    }

    /// Whether this Mac didn't reach the network itself, less than
    /// [`OFFLINE_FOR`] before `at`.
    pub fn offline(&self, at: Instant) -> bool {
        self.state()
            .offline
            .is_some_and(|since| at.saturating_duration_since(since) < OFFLINE_FOR)
    }

    /// This Mac didn't reach the network itself, `at`.
    pub fn note_offline(&self, at: Instant) {
        self.state().offline = Some(at);
    }

    /// The containers that ran before a restart, `names`, which it starts
    /// again ([`Self::left`]).
    pub fn leave(&self, names: Vec<String>) {
        self.state().left = names;
    }

    /// The containers a restart hasn't started again yet.
    pub fn left(&self) -> Vec<String> {
        self.state().left.clone()
    }

    /// The container `name` runs again, or is gone.
    pub fn started(&self, name: &str) {
        self.state().left.retain(|n| n != name);
    }

    /// A session starts with the host's fctl-agent `agent` and `images`: the
    /// fresh sandboxes the host starts by itself go by them too.
    pub fn remember(&self, agent: &Path, images: &[String]) {
        self.state().fresh = Some((agent.to_owned(), images.to_vec()));
    }

    /// The fctl-agent and images a fresh sandbox goes by: `agent` and
    /// `images`, then those the last session that started went by.
    pub fn fresh(&self, agent: Option<&Path>, images: &[String]) -> (Option<PathBuf>, Vec<String>) {
        let state = self.state();
        let (last_agent, last_images) = match &state.fresh {
            Some((a, i)) => (Some(a.clone()), i.as_slice()),
            None => (None, &[][..]),
        };
        let mut all: Vec<String> = Vec::new();
        for image in images.iter().chain(last_images) {
            if !all.contains(image) {
                all.push(image.clone());
            }
        }
        (agent.map(Path::to_owned).or(last_agent), all)
    }

    /// When the host last restarted Apple's `container` for the network.
    pub fn restarted(&self) -> Option<Instant> {
        self.state().restarted
    }

    /// It restarted Apple's `container` for the network, `at`.
    pub fn note_restart(&self, at: Instant) {
        self.state().restarted = Some(at);
    }

    /// Whether the host may restart Apple's `container` `at`, having last
    /// done so for the network `every` ago or longer, or never.
    pub fn may_restart(&self, at: Instant, every: Duration) -> bool {
        self.state()
            .restarted
            .is_none_or(|last| at.saturating_duration_since(last) >= every)
    }

    /// Whether it is time to look again, `at`, having looked `every` ago or
    /// longer, or never; and that the host looks now, when it is.
    pub fn look_now(&self, at: Instant, every: Duration) -> bool {
        let mut state = self.state();
        let due = state
            .looked
            .is_none_or(|last| at.saturating_duration_since(last) >= every);
        if due {
            state.looked = Some(at);
        }
        due
    }

    /// The sandbox `name` didn't reach the network, `at`, while a fresh one
    /// did.
    pub fn note_own_doing(&self, name: &str, at: Instant) {
        let mut state = self.state();
        state
            .own_doing
            .retain(|_, since| at.saturating_duration_since(*since) < RESTART_EVERY);
        state.own_doing.insert(name.to_owned(), at);
    }

    /// Whether the sandbox `name` keeps itself off the network, as the host
    /// found less than [`RESTART_EVERY`] before `at`: it doesn't ask it.
    pub fn own_doing(&self, name: &str, at: Instant) -> bool {
        self.state()
            .own_doing
            .get(name)
            .is_some_and(|since| at.saturating_duration_since(*since) < RESTART_EVERY)
    }

    /// The host looked, `at`, on its own or as a session started: the next
    /// look by itself is [`LOOK_EVERY`] later.
    pub fn note_look(&self, at: Instant) {
        self.state().looked = Some(at);
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The host's fctl-agent `agent`, copied alone into `dir`, which a fresh
/// sandbox mounts: nothing else of the Mac's reaches the sandbox, whose
/// image may be a project's. Copied again once it changed.
pub(crate) fn agent_copy(agent: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    let copy = dir.join("fctl-agent");
    let bytes = std::fs::read(agent)?;
    if std::fs::read(&copy).ok().as_deref() != Some(bytes.as_slice()) {
        std::fs::create_dir_all(dir)?;
        let new = dir.join("fctl-agent.new");
        std::fs::write(&new, &bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&new, &copy)?;
    }
    Ok(copy)
}

/// Whether this machine reaches `target` itself within `within`: why not,
/// when it doesn't.
pub async fn reach_from_here(target: &str, within: Duration) -> Result<(), String> {
    let host = reach::host_of(target);
    let attempt = async {
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host(target)
            .await
            .map_err(|e| format!("{host} didn't resolve: {e}"))?
            .collect();
        let mut failed = Vec::new();
        for addr in reach::order(addrs) {
            match tokio::time::timeout(PER_ADDRESS, tokio::net::TcpStream::connect(addr)).await {
                Ok(Ok(_)) => return Ok(()),
                Ok(Err(e)) => failed.push(format!("{addr}: {e}")),
                Err(_) => failed.push(format!("{addr}: no answer")),
            }
        }
        Err(if failed.is_empty() {
            format!("{host} resolved to no address")
        } else {
            format!("connecting to {host} failed ({})", failed.join("; "))
        })
    };
    tokio::time::timeout(within, attempt)
        .await
        .unwrap_or_else(|_| {
            Err(format!(
                "{host} didn't answer within {}",
                super::stuck::words(within)
            ))
        })
}

/// What the session's log, and the host's, say as the host restarts Apple's
/// `container`, whose sandboxes don't reach `target` (`why`) while this
/// Mac does.
pub(crate) fn restarting_line(target: &str, why: &str) -> String {
    format!(
        "Sandboxes on this runner don't reach {}, though the runner does ({why}): the network Apple's `container` gives them stopped working. The host restarts Apple's `container`, which makes it again: that stops every sandbox on the runner, and starts again those that ran, without what ran in them",
        reach::host_of(target)
    )
}

/// What they say once sandboxes reach `target` again after a restart.
pub(crate) fn restarted_line(target: &str) -> String {
    format!(
        "Restarted Apple's `container`: sandboxes reach {} again",
        reach::host_of(target)
    )
}

/// What a session's log says of its sandbox, which doesn't reach `target`
/// (`why`) while a fresh one does.
pub(crate) fn own_doing_line(target: &str, why: &str) -> String {
    format!(
        "This sandbox doesn't reach {} ({why}), though a new sandbox on this runner does: something in it keeps it from the network, its /etc/hosts say",
        reach::host_of(target)
    )
}

/// Why a session's sandbox can't start for now: sandboxes don't reach the
/// network, or the sandbox didn't start again after a restart of Apple's
/// `container`. The session goes back to the queue rather than fail.
#[derive(Debug)]
pub struct Down(pub String);

impl std::fmt::Display for Down {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Down {}

/// Why a session waits whose sandbox `name` didn't start again after the
/// host restarted Apple's `container` (`why`).
pub(crate) fn not_started_line(name: &str, why: &str) -> String {
    format!(
        "{name} didn't start again after the host restarted Apple's `container` ({why}); the host starts it again as soon as it can"
    )
}

/// What a session's log says when neither its sandbox nor this Mac reaches
/// `target`: restarting Apple's `container` wouldn't help.
pub(crate) fn offline_line(target: &str, why: &str) -> String {
    format!(
        "Neither this sandbox nor its runner reaches {} ({why}): the runner's own network keeps it from it",
        reach::host_of(target)
    )
}

/// Why the driver isn't ready while sandboxes don't reach `target` (`why`)
/// though this Mac does: restarting Apple's `container` `restarted` ago
/// didn't bring their network back, or (`failed`) restarting it failed.
pub(crate) fn down_line(
    target: &str,
    why: &str,
    restarted: Option<Duration>,
    failed: Option<&str>,
) -> String {
    let host = reach::host_of(target);
    let tried = match (failed, restarted) {
        (Some(e), _) => format!("restarting Apple's `container` failed ({e})"),
        (None, Some(ago)) => format!(
            "restarting Apple's `container` {} didn't bring their network back",
            ago_words(ago)
        ),
        (None, None) => "the host hasn't restarted Apple's `container` yet".into(),
    };
    format!(
        "its sandboxes don't reach {host}, though this Mac does ({why}), and {tried}. The host looks again every minute, and restarts Apple's `container` again {} after the last time. A VPN, or a program that holds port 53, can keep sandboxes from the network; restarting the Mac brings it back",
        super::stuck::words(RESTART_EVERY)
    )
}

/// `just now`, `5 minutes ago`.
fn ago_words(d: Duration) -> String {
    match d.as_secs() / 60 {
        0 => "just now".into(),
        1 => "a minute ago".into(),
        m => format!("{m} minutes ago"),
    }
}

/// Why the driver isn't ready while the host restarts Apple's `container`.
pub(crate) fn restarting_now(target: &str) -> String {
    format!(
        "the host is restarting Apple's `container`, whose sandboxes stopped reaching {}",
        reach::host_of(target)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restarts_at_most_once_every_half_hour() {
        let net = Network::default();
        let start = Instant::now();
        assert!(net.may_restart(start, RESTART_EVERY));
        net.note_restart(start);
        assert!(!net.may_restart(start + Duration::from_secs(29 * 60), RESTART_EVERY));
        assert!(net.may_restart(start + RESTART_EVERY, RESTART_EVERY));
        assert_eq!(net.restarted(), Some(start));
    }

    #[test]
    fn sandboxes_that_keep_themselves_off_are_left_for_half_an_hour() {
        let net = Network::default();
        let start = Instant::now();
        assert!(!net.own_doing("fctl-ses-1", start));
        net.note_own_doing("fctl-ses-1", start);
        assert!(net.own_doing("fctl-ses-1", start + Duration::from_secs(29 * 60)));
        assert!(!net.own_doing("fctl-ses-1", start + RESTART_EVERY));
        assert!(!net.own_doing("fctl-ses-2", start));
    }

    #[test]
    fn a_mac_off_the_network_is_remembered_for_five_minutes() {
        let net = Network::default();
        let start = Instant::now();
        assert!(!net.offline(start));
        net.note_offline(start);
        assert!(net.offline(start + Duration::from_secs(4 * 60)));
        assert!(!net.offline(start + OFFLINE_FOR));
    }

    #[test]
    fn fresh_sandboxes_go_by_what_the_last_session_did() -> anyhow::Result<()> {
        let net = Network::default();
        let own = ["ghcr.io/hyperstructure/sandbox:1".to_owned()];
        assert_eq!(net.fresh(None, &own), (None, own.to_vec()));
        let project = "ghcr.io/acme/app:dev".to_owned();
        net.remember(
            Path::new("/opt/fctl/fctl-agent-1"),
            &[project.clone(), own[0].clone()],
        );
        assert_eq!(
            net.fresh(None, &own),
            (
                Some(PathBuf::from("/opt/fctl/fctl-agent-1")),
                vec![own[0].clone(), project]
            )
        );
        let (agent, _) = net.fresh(Some(Path::new("/opt/fctl/fctl-agent-2")), &own);
        assert_eq!(agent, Some(PathBuf::from("/opt/fctl/fctl-agent-2")));
        Ok(())
    }

    #[test]
    fn fresh_sandboxes_get_the_agent_alone() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let agent = tmp.path().join("fctl-agent-aarch64-linux");
        std::fs::write(&agent, "v1")?;
        std::fs::write(tmp.path().join("secret"), "s")?;
        let dir = tmp.path().join("data/reach");
        let copy = agent_copy(&agent, &dir)?;
        assert_eq!(std::fs::read_to_string(&copy)?, "v1");
        // An update is copied in its place.
        std::fs::write(&agent, "v2")?;
        assert_eq!(std::fs::read_to_string(agent_copy(&agent, &dir)?)?, "v2");
        let names: Vec<_> = std::fs::read_dir(&dir)?
            .map(|e| e.map(|e| e.file_name()))
            .collect::<Result<_, _>>()?;
        assert_eq!(names, ["fctl-agent"]);
        Ok(())
    }

    #[test]
    fn looks_once_a_minute() {
        let net = Network::default();
        let start = Instant::now();
        assert!(net.look_now(start, LOOK_EVERY));
        assert!(!net.look_now(start + Duration::from_secs(59), LOOK_EVERY));
        assert!(net.look_now(start + LOOK_EVERY, LOOK_EVERY));
    }

    #[test]
    fn says_why_sandboxes_wait() {
        let why = "github.com didn't resolve: Try again";
        assert_eq!(
            down_line(
                reach::TARGET,
                why,
                Some(Duration::from_secs(4 * 60 + 10)),
                None
            ),
            "its sandboxes don't reach github.com, though this Mac does (github.com didn't resolve: Try again), and restarting Apple's `container` 4 minutes ago didn't bring their network back. The host looks again every minute, and restarts Apple's `container` again 30 minutes after the last time. A VPN, or a program that holds port 53, can keep sandboxes from the network; restarting the Mac brings it back"
        );
        assert!(
            down_line(reach::TARGET, why, Some(Duration::from_secs(5)), None)
                .contains("restarting Apple's `container` just now didn't")
        );
        assert!(
            down_line(reach::TARGET, why, None, Some("`container system stop` did not finish"))
                .contains("and restarting Apple's `container` failed (`container system stop` did not finish).")
        );
    }

    #[tokio::test]
    async fn this_mac_reaches_what_answers() -> anyhow::Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let target = listener.local_addr()?.to_string();
        assert_eq!(reach_from_here(&target, REACH_WITHIN).await, Ok(()));
        drop(listener);
        let why = reach_from_here(&target, REACH_WITHIN)
            .await
            .err()
            .unwrap_or_default();
        assert!(why.starts_with("connecting to 127.0.0.1 failed"), "{why}");
        Ok(())
    }
}
