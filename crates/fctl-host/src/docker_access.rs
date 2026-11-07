//! Reaching Docker, whose socket lets in root and, on Linux, the members of
//! its group (`docker`). People join that group once they have installed
//! Docker, but a process keeps the groups it started with: their shell until
//! they log in again, and the systemd user manager that runs the host
//! service until the machine restarts. So this says what stands between
//! fctl and Docker, with the command that fixes it, and `fctl host` takes
//! the group by itself: `sg` runs a command in a group its user is a member
//! of without asking for a password, and `fctl host` starts again through
//! it ([`regroup`]).

use std::ffi::OsString;
use std::path::PathBuf;

/// Where the docker CLI finds the daemon when `DOCKER_HOST` doesn't say.
pub const DEFAULT_SOCKET: &str = "/var/run/docker.sock";

/// Set in the environment of an fctl that `sg` started with the socket's
/// group: the process id that ran `sg`. It keeps that fctl from starting
/// again, and ties it to `sg` when `sg` waits as its parent.
pub const REGROUPED: &str = "FCTL_DOCKER_REGROUPED";

/// Why the docker CLI couldn't reach the daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The socket is there, and refuses this process.
    Denied,
    /// Nothing answers at the socket.
    NotRunning,
}

/// Why the docker CLI couldn't reach the daemon, from what it printed;
/// `None` when it failed for another reason. Docker 28 and earlier say
/// "the Docker daemon socket" and "Cannot connect to the Docker daemon",
/// later versions "the docker API".
pub fn refusal(output: &str) -> Option<Refusal> {
    let o = output.to_ascii_lowercase();
    if o.contains("permission denied while trying to connect to the docker")
        || (o.contains("permission denied") && o.contains("docker.sock"))
    {
        Some(Refusal::Denied)
    } else if o.contains("cannot connect to the docker daemon")
        || o.contains("is the docker daemon running")
        || (o.contains("failed to connect to the docker api") && o.contains("daemon is running"))
    {
        Some(Refusal::NotRunning)
    } else {
        None
    }
}

/// What decides why the socket refuses this process, and what fixes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// The socket the docker CLI connects to, when it is on this machine.
    pub socket: Option<PathBuf>,
    /// The socket's group, unless it is root's.
    pub group: Option<String>,
    /// This process's user.
    pub user: Option<String>,
    /// Whether `user` is a member of `group`, as the group database says.
    pub member: bool,
    /// Whether this process has `group`.
    pub has_group: bool,
    /// Whether `sg` is here to start a command in `group`.
    pub sg: bool,
    /// Whether `sg` started this process in `group` already ([`REGROUPED`]).
    pub regrouped: bool,
    /// How to start the daemon here, when it isn't running.
    pub start: Option<&'static str>,
}

impl Facts {
    /// The group to start again in: the socket's, which this process's user
    /// is a member of but this process doesn't have, when `sg` is here and
    /// hasn't started it already.
    pub fn group_to_take(&self) -> Option<&str> {
        if self.member && !self.has_group && self.sg && !self.regrouped {
            self.group.as_deref()
        } else {
            None
        }
    }
}

/// What stands between fctl and Docker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The daemon isn't running; `start` says how to start it, where fctl
    /// knows.
    NotRunning { start: Option<&'static str> },
    /// `user` isn't in `group`, whose members the socket lets in.
    NotInGroup { user: String, group: String },
    /// `user` is in `group`, but joined it after this process started.
    /// `fctl host` takes the group as it starts, with `sg` when there is
    /// one.
    NotYet {
        user: String,
        group: String,
        sg: bool,
    },
    /// The socket refuses this process for another reason.
    Denied { socket: Option<PathBuf> },
}

impl Problem {
    pub fn of(refusal: Refusal, facts: &Facts) -> Self {
        match (refusal, &facts.user, &facts.group) {
            (Refusal::NotRunning, ..) => Self::NotRunning { start: facts.start },
            (Refusal::Denied, Some(user), Some(group)) if !facts.member => Self::NotInGroup {
                user: user.clone(),
                group: group.clone(),
            },
            (Refusal::Denied, Some(user), Some(group))
                if !facts.has_group && !(facts.sg && facts.regrouped) =>
            {
                Self::NotYet {
                    user: user.clone(),
                    group: group.clone(),
                    sg: facts.sg,
                }
            }
            (Refusal::Denied, ..) => Self::Denied {
                socket: facts.socket.clone(),
            },
        }
    }

    /// The command that adds the user to the group, when that is the fix.
    pub fn join_command(&self) -> Option<String> {
        match self {
            Self::NotInGroup { user, group } => Some(format!("sudo usermod -aG {group} {user}")),
            _ => None,
        }
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRunning { start: Some(start) } => {
                write!(f, "the Docker daemon isn't running: {start}")
            }
            Self::NotRunning { start: None } => f.write_str("the Docker daemon isn't running"),
            Self::NotInGroup { user, group } => write!(
                f,
                "can't connect (permission denied): {user} isn't in the {group} group: `sudo usermod -aG {group} {user}`, then `fctl host restart`"
            ),
            Self::NotYet {
                user,
                group,
                sg: true,
            } => write!(
                f,
                "can't connect (permission denied): {user} joined the {group} group after fctl started: `fctl host restart` takes the group"
            ),
            Self::NotYet {
                user,
                group,
                sg: false,
            } => write!(
                f,
                "can't connect (permission denied): {user} joined the {group} group after fctl started, and there is no `sg` to take it: restart the machine"
            ),
            Self::Denied {
                socket: Some(socket),
            } => {
                write!(
                    f,
                    "can't connect to {} (permission denied)",
                    socket.display()
                )
            }
            Self::Denied { socket: None } => f.write_str("can't connect (permission denied)"),
        }
    }
}

/// A docker command that couldn't reach the daemon.
#[derive(Debug)]
pub struct Unreachable(pub Problem);

impl Unreachable {
    /// What the docker CLI printed as an [`Unreachable`], with this
    /// machine's [`Facts`]; `None` when it failed for another reason.
    pub fn from_output(output: &str) -> Option<Self> {
        refusal(output).map(|r| Self(Problem::of(r, &facts())))
    }
}

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fctl can't use Docker: {}", self.0)
    }
}

impl std::error::Error for Unreachable {}

/// The socket the docker CLI connects to with `DOCKER_HOST` set to
/// `docker_host`; `None` when it isn't one on this machine.
pub fn socket(docker_host: Option<&str>) -> Option<PathBuf> {
    match docker_host.filter(|h| !h.is_empty()) {
        Some(host) => host.strip_prefix("unix://").map(PathBuf::from),
        None => Some(PathBuf::from(DEFAULT_SOCKET)),
    }
}

/// This machine's [`Facts`].
pub fn facts() -> Facts {
    let socket = socket(std::env::var("DOCKER_HOST").ok().as_deref());
    #[cfg(target_os = "linux")]
    {
        linux::facts(socket)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Facts {
            socket,
            start: cfg!(target_os = "macos").then_some("start Docker Desktop, OrbStack or Colima"),
            ..Facts::default()
        }
    }
}

/// The command that restarts this user's systemd manager, which then
/// starts the host service again with the groups the user has now.
pub fn restart_manager() -> String {
    #[cfg(unix)]
    let uid = nix::unistd::getuid().as_raw().to_string();
    #[cfg(not(unix))]
    let uid = "$(id -u)";
    format!("sudo systemctl restart user@{uid}.service")
}

/// Where Docker Engine's install guide for this Linux distribution is, from
/// `/etc/os-release` (its `ID`, then its `ID_LIKE`).
pub fn install_guide(os_release: &str) -> String {
    const GUIDES: &[(&str, &str)] = &[
        ("ubuntu", "ubuntu"),
        ("debian", "debian"),
        ("raspbian", "raspberry-pi-os"),
        ("fedora", "fedora"),
        ("centos", "centos"),
        ("rhel", "rhel"),
    ];
    let field = |key: &str| {
        os_release.lines().find_map(|l| {
            let v = l.trim().strip_prefix(key)?.strip_prefix('=')?;
            Some(v.trim().trim_matches(['"', '\'']).to_ascii_lowercase())
        })
    };
    let ids = field("ID").into_iter().chain(
        field("ID_LIKE")
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    );
    for id in ids {
        if let Some((_, page)) = GUIDES.iter().find(|(known, _)| *known == id) {
            return format!("https://docs.docker.com/engine/install/{page}/");
        }
    }
    "https://docs.docker.com/engine/install/".into()
}

/// The `sh` command that runs `words` in its own place, each word quoted.
pub fn exec_line(words: &[OsString]) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let mut line = b"exec".to_vec();
        for word in words {
            line.extend_from_slice(b" '");
            for &b in word.as_bytes() {
                if b == b'\'' {
                    line.extend_from_slice(b"'\\''");
                } else {
                    line.push(b);
                }
            }
            line.push(b'\'');
        }
        OsString::from_vec(line)
    }
    #[cfg(not(unix))]
    {
        let mut line = OsString::from("exec");
        for word in words {
            line.push(" ");
            line.push(word);
        }
        line
    }
}

/// Starts this fctl again through `sg`, in the Docker socket's group, when
/// the socket refuses it but its user is a member of that group: they
/// joined it after this process, or the service manager that started it,
/// started. Returns when it doesn't start again: there is no need, or no
/// way. An fctl that `sg` started stops when `sg` does: shadow-utils' `sg`
/// (Ubuntu's, Fedora's, Arch's) waits for the command as its parent, and
/// systemd, which knows only `sg`, stops only `sg`. util-linux's `sg`
/// (Debian's since 13) runs the command in its own place.
pub fn regroup() {
    #[cfg(target_os = "linux")]
    linux::regroup();
}

/// What stands between this process and Docker, as the socket answers it
/// now: `None` when it lets this process in, or when it isn't a socket on
/// this machine.
pub fn problem() -> Option<Problem> {
    #[cfg(unix)]
    {
        use std::io::ErrorKind;
        let path = socket(std::env::var("DOCKER_HOST").ok().as_deref())?;
        let refusal = match std::os::unix::net::UnixStream::connect(&path) {
            Ok(_) => return None,
            Err(e) if e.kind() == ErrorKind::PermissionDenied => Refusal::Denied,
            Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => {
                Refusal::NotRunning
            }
            Err(_) => return None,
        };
        Some(Problem::of(refusal, &facts()))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Whether the process `pid` has the Docker socket's group: the host
/// service's, which takes it as it starts. `None` where that can't be
/// told: the socket isn't of a group other than root's, or there is no
/// such process.
pub fn process_has_group(pid: u32) -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        linux::process_has_group(pid)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// The groups a process has, from its `/proc/<pid>/status`: its effective
/// group and its supplementary groups.
pub fn status_groups(status: &str) -> Vec<u32> {
    let field = |key: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .map(|v| v.split_whitespace().filter_map(|g| g.parse::<u32>().ok()))
    };
    let effective = field("Gid:").and_then(|mut ids| ids.nth(1));
    effective
        .into_iter()
        .chain(field("Groups:").into_iter().flatten())
        .collect()
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{Facts, REGROUPED, exec_line};
    use nix::sys::signal::Signal;
    use nix::unistd::{Gid, Group, User, getegid, getgroups, getppid, getuid};
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Stdio;

    /// Whether the socket at `path` refuses this process.
    fn denied(path: &Path) -> bool {
        std::os::unix::net::UnixStream::connect(path)
            .is_err_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
    }

    /// How `sg` started this process, from [`REGROUPED`]: in its own place
    /// (`InPlace`), or as its child, which it waits for (`Child`).
    #[derive(Debug, PartialEq, Eq)]
    enum Started {
        InPlace,
        Child(i32),
        No,
    }

    fn started() -> Started {
        let Some(ran) = std::env::var(REGROUPED)
            .ok()
            .and_then(|v| v.trim().parse::<i32>().ok())
        else {
            return Started::No;
        };
        // Processes this one starts inherit the variable, and aren't it.
        if i32::try_from(std::process::id()) == Ok(ran) {
            Started::InPlace
        } else if getppid().as_raw() == ran {
            Started::Child(ran)
        } else {
            Started::No
        }
    }

    pub fn facts(socket: Option<PathBuf>) -> Facts {
        // A socket of root's group lets in no group one should join.
        let gid = socket
            .as_deref()
            .and_then(|s| std::fs::metadata(s).ok())
            .map(|m| Gid::from_raw(m.gid()))
            .filter(|g| g.as_raw() != 0);
        let group = gid.and_then(|g| Group::from_gid(g).ok().flatten());
        let user = User::from_uid(getuid()).ok().flatten();
        let member = match (&group, &user) {
            (Some(g), Some(u)) => u.gid == g.gid || g.mem.contains(&u.name),
            _ => false,
        };
        let has_group = gid
            .is_some_and(|g| getegid() == g || getgroups().is_ok_and(|groups| groups.contains(&g)));
        let systemd = Path::new("/run/systemd/system").is_dir()
            && socket.as_deref() == Some(Path::new(super::DEFAULT_SOCKET));
        Facts {
            socket,
            group: group.map(|g| g.name),
            user: user.map(|u| u.name),
            member,
            has_group,
            sg: crate::driver::process::which("sg").is_some(),
            regrouped: started() != Started::No,
            start: systemd.then_some(
                "`sudo systemctl enable --now docker` starts it, now and whenever the machine starts",
            ),
        }
    }

    pub fn process_has_group(pid: u32) -> Option<bool> {
        let socket = super::socket(std::env::var("DOCKER_HOST").ok().as_deref())?;
        let gid = std::fs::metadata(socket).ok()?.gid();
        if gid == 0 {
            return None;
        }
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        Some(super::status_groups(&status).contains(&gid))
    }

    pub fn regroup() {
        match started() {
            Started::InPlace => return,
            Started::Child(sg) => {
                // Stop with `sg`; if it ended before this could, it is gone
                // already, and so is the service it was.
                let _ = nix::sys::prctl::set_pdeathsig(Signal::SIGTERM);
                if getppid().as_raw() != sg {
                    std::process::exit(0);
                }
                return;
            }
            Started::No => {}
        }
        let facts = facts(super::socket(std::env::var("DOCKER_HOST").ok().as_deref()));
        let Some(socket) = facts.socket.as_deref() else {
            return;
        };
        if !denied(socket) {
            return;
        }
        let Some(group) = facts.group_to_take() else {
            return;
        };
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let mut words = vec![exe.into_os_string()];
        words.extend(std::env::args_os().skip(1));
        // `sg` asks members for no password; one that fails anyway must not
        // make the service start again and again.
        let works = std::process::Command::new("sg")
            .args([group, "-c", "true"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !works {
            tracing::warn!(
                "{} is in the {group} group, which may use Docker, but this process isn't, and `sg {group}` fails",
                facts.user.as_deref().unwrap_or("this user")
            );
            return;
        }
        tracing::info!(
            "starting again in the {group} group, which may use Docker: {} joined it after this process started",
            facts.user.as_deref().unwrap_or("this user")
        );
        let e = std::process::Command::new("sg")
            .arg(group)
            .arg("-c")
            .arg(exec_line(&words))
            .env(REGROUPED, std::process::id().to_string())
            .exec();
        tracing::warn!("starting again through sg: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DENIED_28: &str = "docker version failed: permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock: Get \"http://%2Fvar%2Frun%2Fdocker.sock/v1.47/version\": dial unix /var/run/docker.sock: connect: permission denied";
    const DENIED_29: &str = "docker ps failed: permission denied while trying to connect to the docker API at unix:///var/run/docker.sock";
    const STOPPED_28: &str = "docker version failed: Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?";
    const STOPPED_29: &str = "docker inspect failed: failed to connect to the docker API at unix:///var/run/docker.sock; check if the path is correct and if the daemon is running: dial unix /var/run/docker.sock: connect: no such file or directory";

    #[test]
    fn tells_a_refusal_from_other_failures() {
        assert_eq!(refusal(DENIED_28), Some(Refusal::Denied));
        assert_eq!(refusal(DENIED_29), Some(Refusal::Denied));
        assert_eq!(refusal(STOPPED_28), Some(Refusal::NotRunning));
        assert_eq!(refusal(STOPPED_29), Some(Refusal::NotRunning));
        for other in [
            "docker pull failed: Error response from daemon: manifest unknown",
            "docker inspect failed: Error: No such object: fctl-ses-01",
            "docker run failed: permission denied: /home/ada/project",
            "",
        ] {
            assert_eq!(refusal(other), None, "{other}");
        }
    }

    fn alice() -> Facts {
        Facts {
            socket: Some(PathBuf::from(DEFAULT_SOCKET)),
            group: Some("docker".into()),
            user: Some("alice".into()),
            member: false,
            has_group: false,
            sg: true,
            regrouped: false,
            start: Some("`sudo systemctl enable --now docker` starts it"),
        }
    }

    #[test]
    fn says_what_fixes_each_problem() {
        let outside = Problem::of(Refusal::Denied, &alice());
        assert_eq!(
            outside.to_string(),
            "can't connect (permission denied): alice isn't in the docker group: `sudo usermod -aG docker alice`, then `fctl host restart`"
        );
        assert_eq!(
            outside.join_command().as_deref(),
            Some("sudo usermod -aG docker alice")
        );
        // A member whose process lacks the group gets it through `sg` as
        // `fctl host` starts; without `sg`, only a new login or a restart
        // gives it.
        let member = Facts {
            member: true,
            ..alice()
        };
        assert_eq!(
            Problem::of(Refusal::Denied, &member).to_string(),
            "can't connect (permission denied): alice joined the docker group after fctl started: `fctl host restart` takes the group"
        );
        // Started through `sg` already, and refused all the same.
        assert_eq!(
            Problem::of(
                Refusal::Denied,
                &Facts {
                    regrouped: true,
                    ..member.clone()
                }
            ),
            Problem::Denied {
                socket: Some(PathBuf::from(DEFAULT_SOCKET))
            }
        );
        let no_sg = Problem::of(
            Refusal::Denied,
            &Facts {
                sg: false,
                ..member
            },
        );
        assert_eq!(
            no_sg.to_string(),
            "can't connect (permission denied): alice joined the docker group after fctl started, and there is no `sg` to take it: restart the machine"
        );
        assert_eq!(no_sg.join_command(), None);
        // A socket of root's group, or one fctl can't look at.
        let unknown = Facts {
            group: None,
            ..alice()
        };
        assert_eq!(
            Problem::of(Refusal::Denied, &unknown).to_string(),
            "can't connect to /var/run/docker.sock (permission denied)"
        );
        assert_eq!(
            Problem::of(Refusal::NotRunning, &alice()).to_string(),
            "the Docker daemon isn't running: `sudo systemctl enable --now docker` starts it"
        );
        assert_eq!(
            Problem::of(Refusal::NotRunning, &Facts::default()).to_string(),
            "the Docker daemon isn't running"
        );
        assert_eq!(
            Unreachable(outside).to_string(),
            "fctl can't use Docker: can't connect (permission denied): alice isn't in the docker group: `sudo usermod -aG docker alice`, then `fctl host restart`"
        );
    }

    #[test]
    fn takes_the_group_only_when_it_is_a_members_and_sg_can_give_it() {
        let member = Facts {
            member: true,
            ..alice()
        };
        assert_eq!(member.group_to_take(), Some("docker"));
        for no in [
            alice(),
            Facts {
                has_group: true,
                ..member.clone()
            },
            Facts {
                sg: false,
                ..member.clone()
            },
            Facts {
                regrouped: true,
                ..member.clone()
            },
            Facts {
                group: None,
                ..member.clone()
            },
        ] {
            assert_eq!(no.group_to_take(), None, "{no:?}");
        }
    }

    #[test]
    fn reads_the_groups_of_a_process() {
        let status = "Name:\tfctl\nUmask:\t0022\nUid:\t1000\t1000\t1000\t1000\nGid:\t1000\t999\t1000\t1000\nFDSize:\t64\nGroups:\t4 24 27 \nNStgid:\t4242\n";
        assert_eq!(status_groups(status), [999, 4, 24, 27]);
        // No supplementary groups.
        assert_eq!(
            status_groups("Gid:\t1000\t1000\t1000\t1000\nGroups:\t\n"),
            [1000]
        );
        assert!(status_groups("").is_empty());
    }

    #[test]
    fn finds_the_socket_the_cli_uses() {
        assert_eq!(socket(None), Some(PathBuf::from("/var/run/docker.sock")));
        assert_eq!(
            socket(Some("")),
            Some(PathBuf::from("/var/run/docker.sock"))
        );
        assert_eq!(
            socket(Some("unix:///run/user/1000/docker.sock")),
            Some(PathBuf::from("/run/user/1000/docker.sock"))
        );
        assert_eq!(socket(Some("tcp://10.0.0.2:2376")), None);
        assert_eq!(socket(Some("ssh://ada@build")), None);
    }

    #[test]
    fn links_the_install_guide_of_the_distribution() {
        let guide = |s: &str| install_guide(s);
        assert_eq!(
            guide("PRETTY_NAME=\"Ubuntu 24.04 LTS\"\nID=ubuntu\nID_LIKE=debian\n"),
            "https://docs.docker.com/engine/install/ubuntu/"
        );
        assert_eq!(
            guide("ID=debian\nVERSION_ID=\"13\"\n"),
            "https://docs.docker.com/engine/install/debian/"
        );
        // Derivatives follow what they are like.
        assert_eq!(
            guide("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"),
            "https://docs.docker.com/engine/install/ubuntu/"
        );
        assert_eq!(
            guide("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n"),
            "https://docs.docker.com/engine/install/rhel/"
        );
        assert_eq!(
            guide("ID=raspbian\nID_LIKE=debian\n"),
            "https://docs.docker.com/engine/install/raspberry-pi-os/"
        );
        assert_eq!(
            guide("ID=arch\n"),
            "https://docs.docker.com/engine/install/"
        );
        assert_eq!(guide(""), "https://docs.docker.com/engine/install/");
    }

    #[cfg(unix)]
    #[test]
    fn quotes_each_word_for_sh() -> anyhow::Result<()> {
        let words: Vec<OsString> = [
            "/home/ada/.local/bin/fctl",
            "host",
            "install",
            "--name",
            "ada's box",
            "--image",
            "$HOME `x`",
        ]
        .map(OsString::from)
        .to_vec();
        let line = exec_line(&words);
        assert_eq!(
            line.to_string_lossy(),
            "exec '/home/ada/.local/bin/fctl' 'host' 'install' '--name' 'ada'\\''s box' '--image' '$HOME `x`'"
        );
        // sh gives the words back as they were.
        let mut echo = vec![OsString::from("printf"), OsString::from("%s\\n")];
        echo.extend(words.iter().cloned());
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(exec_line(&echo))
            .output()?;
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "/home/ada/.local/bin/fctl\nhost\ninstall\n--name\nada's box\n--image\n$HOME `x`\n"
        );
        Ok(())
    }
}
