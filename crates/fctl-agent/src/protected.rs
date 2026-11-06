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
//!    reaches that one through a link in place of its own socket;
//! 4. moves the journal and state of such a supervisor, once it has gone,
//!    to its own directory;
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
    pub lock: File,
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
    if held(&old_socket.with_extension("lock")) {
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

/// Whether another process holds the lock at `path`.
fn held(path: &Path) -> bool {
    let Ok(file) = File::options().write(true).open(path) else {
        return false;
    };
    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
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
/// sandbox's host was updated. The GitHub tokens stay.
fn migrate(from: &Path, to: &Path) {
    let journal = OWN_STATE[0];
    if to.join(journal).exists() || !from.join(journal).exists() {
        return;
    }
    for name in OWN_STATE {
        let (old, new) = (from.join(name), to.join(name));
        if !old.exists() {
            continue;
        }
        let moved = std::fs::rename(&old, &new)
            .or_else(|_| std::fs::copy(&old, &new).and_then(|_| std::fs::remove_file(&old)));
        match moved.and_then(|()| std::os::unix::fs::chown(&new, Some(0), Some(0))) {
            Ok(()) => {}
            Err(e) => tracing::warn!("moving {} to {}: {e}", old.display(), new.display()),
        }
    }
    tracing::info!(
        "moved the session's journal and state from {} to {}",
        from.display(),
        to.display()
    );
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

        // A supervisor with a journal of its own keeps it.
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
        let lock = link::lock(&path)?;
        assert!(lock.is_some());
        assert!(held(&path));
        drop(lock);
        assert!(!held(&path));
        Ok(())
    }
}
