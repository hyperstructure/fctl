//! `fctl-agent serve --user`: the supervisor as root, out of reach of the
//! user it runs Claude as ([`crate::user`]).
//!
//! As root, before it takes the user's identity, it:
//!
//! 1. makes its directories (the socket's, the state's and the log's) ones
//!    only root may enter;
//! 2. takes its lock, or leaves the sandbox to the supervisor that holds it;
//! 3. leaves it to a supervisor from before, which runs as the user from
//!    the user's `~/.fctl` (one that ran as the host was updated): the host
//!    reaches that one through a link in place of its own socket. Only
//!    until a supervisor has served the sandbox as root: from then on the
//!    user's `~/.fctl` has nothing to say about who serves it;
//! 4. moves the journal and state of such a supervisor, once it has gone,
//!    to its own directory: regular files only, which it doesn't follow
//!    out of the user's reach;
//! 5. binds the host's socket;
//! 6. keeps itself from the OOM killer, where it may.

use crate::user::{self, accounts};
use crate::{link, local};
use anyhow::{Context, Result, anyhow, bail};
use std::fs::File;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

/// Where a supervisor that runs as the user keeps its files, in its home:
/// one from before, and the session socket and the GitHub tokens of one
/// that runs as root.
pub const USER_DIR: &str = ".fctl";

/// fctl-agent's own files in a state directory, which a supervisor that
/// ran as the user kept in `~/.fctl/state`, beside the GitHub tokens.
const OWN_STATE: [&str; 3] = ["journal.jsonl", "journal.ack", "state.json"];

/// A supervisor that serves, as the user now.
pub struct Started {
    /// The host's socket, bound.
    pub listener: UnixListener,
    /// Held while it serves.
    pub lock: link::Lock,
    pub socket: PathBuf,
    pub state: PathBuf,
    /// Where the user's `fctl` reaches the supervisor: in its `~/.fctl`.
    pub session_socket: PathBuf,
    /// Where the GitHub tokens and git's credential helper go: the user's
    /// `~/.fctl/state`, which git and `gh` read.
    pub user_state: PathBuf,
}

pub enum Start {
    Serving(Box<Started>),
    /// Another supervisor serves the sandbox.
    Taken,
}

/// Readies the supervisor for the user `spec` names, then takes its
/// identity ([`user::protect`]). Call it as root, before other threads
/// start.
pub fn start(spec: &str, socket: &Path, state: &Path, log: Option<&Path>) -> Result<Start> {
    if !nix::unistd::geteuid().is_root() {
        bail!("fctl-agent serve --user runs as root");
    }
    let user = accounts::find(Some(spec)).map_err(|e| anyhow!(e))?;
    for dir in [socket.parent(), Some(state), log.and_then(Path::parent)]
        .into_iter()
        .flatten()
    {
        private_dir(dir)?;
    }
    let Some(lock) = link::lock(&socket.with_extension("lock"))? else {
        tracing::info!(
            "fctl-agent is already serving {}; this one exits",
            socket.display()
        );
        return Ok(Start::Taken);
    };
    let before = Path::new(&user.home).join(USER_DIR);
    let old_socket = before.join("agent.sock");
    // A supervisor that served as root has its journal here; one from
    // before can't come after it.
    let served = state.join(OWN_STATE[0]).exists();
    if !served && held(&old_socket.with_extension("lock")) {
        point_at(socket, &old_socket)?;
        tracing::info!(
            "an fctl-agent from before serves this sandbox, as its user, at {}: it keeps the session, which the host reaches through {}",
            old_socket.display(),
            socket.display()
        );
        return Ok(Start::Taken);
    }
    migrate(&before.join("state"), state);
    let listener = bind(socket)?;
    match user::shield_from_oom() {
        Ok(()) => tracing::info!("the OOM killer leaves fctl-agent alone"),
        Err(e) => tracing::info!(
            "the OOM killer may stop fctl-agent: the sandbox doesn't let it out of its way ({e})"
        ),
    }
    if user.uid == 0 {
        tracing::warn!("Claude runs as root in this sandbox, so its commands can stop fctl-agent");
    }
    user::protect(&user)?;
    tracing::info!(
        "fctl-agent runs as root, and Claude and its commands as uid {}",
        user.uid
    );
    Ok(Start::Serving(Box::new(Started {
        listener,
        lock,
        socket: socket.to_owned(),
        state: state.to_owned(),
        session_socket: local::socket_path(&old_socket),
        user_state: before.join("state"),
    })))
}

/// Makes `dir`, root's and only root's.
fn private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    std::os::unix::fs::chown(dir, Some(0), Some(0))
        .with_context(|| format!("giving {} to root", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("closing {} to others", dir.display()))?;
    Ok(())
}

/// Whether another process holds the lock at `path`, a regular file the
/// user owns: not a link, and nothing that would keep root waiting.
fn held(path: &Path) -> bool {
    let Ok(file) = open_plain(path) else {
        return false;
    };
    held_through(file)
}

/// [`held`], with the lock's file open.
fn held_through(file: File) -> bool {
    match file.try_lock() {
        // Let go of it before the file closes, for a process forked meanwhile
        // would hold it until it execs ([`link::Lock`]).
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(e) => matches!(e, std::fs::TryLockError::WouldBlock),
    }
}

/// Opens the regular file at `path` for reading and writing, where the
/// user may have put anything: a link isn't followed, and a FIFO or a
/// device is opened without waiting and then refused.
fn open_plain(path: &Path) -> std::io::Result<File> {
    use nix::fcntl::OFlag;
    use std::os::unix::fs::OpenOptionsExt;
    let file = File::options()
        .read(true)
        .write(true)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    Ok(file)
}

/// Puts a link to `target` where the host's socket goes, in place of
/// whatever is there.
fn point_at(socket: &Path, target: &Path) -> Result<()> {
    remove_socket(socket)?;
    std::os::unix::fs::symlink(target, socket)
        .with_context(|| format!("linking {} to {}", socket.display(), target.display()))
}

/// Removes the socket or link at `path`, if there is one.
fn remove_socket(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => bail!("{} is a directory", path.display()),
        Ok(_) => std::fs::remove_file(path).with_context(|| format!("removing {}", path.display())),
        Err(_) => Ok(()),
    }
}

/// Binds the host's socket, for root alone.
fn bind(socket: &Path) -> Result<UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    remove_socket(socket)?;
    let listener =
        UnixListener::bind(socket).with_context(|| format!("binding {}", socket.display()))?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

/// Moves the journal and state a supervisor that ran as the user kept in
/// `from` (`~/.fctl/state`) to `to`, for a session it ran before this
/// sandbox's host was updated. The GitHub tokens stay. The user owns
/// `from`, so only regular files of its own come along, as they are (a
/// link to another file would have root write there).
fn migrate(from: &Path, to: &Path) {
    let journal = OWN_STATE[0];
    if to.join(journal).exists() || !from.join(journal).is_file() {
        return;
    }
    for name in OWN_STATE {
        let (old, new) = (from.join(name), to.join(name));
        if std::fs::symlink_metadata(&old).is_err() {
            continue;
        }
        if let Err(e) = move_plain(&old, &new) {
            tracing::warn!("not moving {} to {}: {e}", old.display(), new.display());
        }
    }
    tracing::info!(
        "moved the session's journal and state from {} to {}",
        from.display(),
        to.display()
    );
}

/// Moves the regular file `old` to `new`, root's then.
fn move_plain(old: &Path, new: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    // A rename moves what is at `old`, link or not; once in root's
    // directory, nothing the user does changes it, so it is checked there.
    if std::fs::rename(old, new).is_err() {
        // Another filesystem: the file's bytes, read without following.
        let mut from = open_plain(old)?;
        let mut to = File::options().write(true).create_new(true).open(new)?;
        std::io::copy(&mut from, &mut to)?;
        std::fs::remove_file(old)?;
    }
    let meta = std::fs::symlink_metadata(new)?;
    if !meta.is_file() || meta.nlink() > 1 {
        std::fs::remove_file(new)?;
        return Err(std::io::Error::other("not a regular file of its own"));
    }
    std::os::unix::fs::lchown(new, Some(0), Some(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_supervisor_from_before_hands_over_its_journal_and_state() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (from, to) = (dir.path().join("home/.fctl/state"), dir.path().join("root"));
        std::fs::create_dir_all(&from)?;
        std::fs::create_dir_all(&to)?;
        for (name, text) in [
            ("journal.jsonl", "{}\n"),
            ("journal.ack", "3"),
            ("state.json", "{}"),
            ("github-token", "ghs_x"),
            ("git-credential-fctl", "#!/bin/sh"),
        ] {
            std::fs::write(from.join(name), text)?;
        }
        // Not as root, the files stay the test's, which is enough here.
        migrate(&from, &to);
        for name in OWN_STATE {
            assert!(to.join(name).is_file(), "{name}");
            assert!(!from.join(name).exists(), "{name}");
        }
        assert_eq!(std::fs::read_to_string(to.join("journal.ack"))?, "3");
        // The tokens stay where git and gh read them.
        assert!(from.join("github-token").is_file());
        assert!(!to.join("github-token").exists());

        // Links and FIFOs the user put there stay out, as do hard links.
        let (from, to) = (dir.path().join("planted"), dir.path().join("root2"));
        std::fs::create_dir_all(&from)?;
        std::fs::create_dir_all(&to)?;
        let elsewhere = dir.path().join("elsewhere");
        std::fs::write(&elsewhere, "someone else's")?;
        std::fs::write(from.join("journal.jsonl"), "{}\n")?;
        std::os::unix::fs::symlink(&elsewhere, from.join("journal.ack"))?;
        std::fs::hard_link(&elsewhere, from.join("state.json"))?;
        migrate(&from, &to);
        assert!(to.join("journal.jsonl").is_file());
        assert!(std::fs::symlink_metadata(to.join("journal.ack")).is_err());
        assert!(std::fs::symlink_metadata(to.join("state.json")).is_err());
        assert_eq!(std::fs::read_to_string(&elsewhere)?, "someone else's");
        let fifo = dir.path().join("fifo");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU)?;
        assert!(open_plain(&fifo).is_err());
        assert!(!held(&fifo));

        // A supervisor with a journal of its own keeps it.
        let (from, to) = (dir.path().join("home/.fctl/state"), dir.path().join("root"));
        std::fs::write(from.join("journal.jsonl"), "older\n")?;
        migrate(&from, &to);
        assert_eq!(std::fs::read_to_string(to.join("journal.jsonl"))?, "{}\n");
        assert!(from.join("journal.jsonl").exists());
        Ok(())
    }

    #[test]
    fn the_host_reaches_a_supervisor_from_before_through_a_link() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (socket, old) = (dir.path().join("agent.sock"), dir.path().join("old.sock"));
        let _old = UnixListener::bind(&old)?;
        // A stale socket goes.
        drop(UnixListener::bind(&socket)?);
        point_at(&socket, &old)?;
        assert_eq!(std::fs::read_link(&socket)?, old);
        assert!(std::os::unix::net::UnixStream::connect(&socket).is_ok());
        // Once that one has gone, the socket takes the link's place.
        use std::os::unix::fs::FileTypeExt;
        let listener = bind(&socket)?;
        assert!(std::fs::symlink_metadata(&socket)?.file_type().is_socket());
        drop(listener);
        assert!(point_at(dir.path(), &old).is_err());
        Ok(())
    }

    #[test]
    fn finds_who_holds_a_lock() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("agent.lock");
        assert!(!held(&path));
        // Finding it free lets go of it, though a process forked meanwhile
        // keeps a copy of the file until it execs.
        std::fs::File::create(&path)?;
        let file = open_plain(&path)?;
        let checked = file.try_clone()?;
        assert!(!held_through(file));
        let Some(lock) = link::lock(&path)? else {
            bail!("nobody else holds the lock");
        };
        assert!(held(&path));
        // A process another test forks keeps a copy of the file until it
        // execs: the lock is let go of all the same.
        let copy = lock.copy()?;
        drop(lock);
        assert!(!held(&path));
        drop((copy, checked));
        Ok(())
    }
}
