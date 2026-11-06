//! `fctl-agent run-as UID GID GROUPS CWD -- PROGRAM ARGS…`: how fctl-agent
//! starts a program as another user. `vm-serve` starts every program the
//! host runs in a microVM with it, and a supervisor that runs as root
//! ([`super::protect`]) the programs it runs for the session: Claude, git,
//! `gh`, the setup script. As root, it takes the user's identity and a
//! session of its own, so that whoever started it can stop the program with
//! everything it started, then becomes the program.

use anyhow::{Context, Result, bail};
use nix::unistd::{Gid, Uid, execvp, getresuid, setgid, setgroups, setresuid, setsid, setuid};
use std::ffi::CString;

pub fn run_as(uid: u32, gid: u32, groups: &str, cwd: &str, argv: &[String]) -> Result<()> {
    let Some(program) = argv.first() else {
        bail!("no program to run");
    };
    // A protected supervisor works as its user with root as its real user
    // ([`super::protect`]): its programs start so, and take root back to
    // become the user, all of it.
    let ids = getresuid().context("reading the user")?;
    if ids.real.is_root() && !ids.effective.is_root() {
        setresuid(Uid::from_raw(0), Uid::from_raw(0), Uid::from_raw(0))
            .context("taking root back")?;
    }
    // The supervisor keeps itself from the OOM killer; its programs don't.
    super::reset_oom_score();
    setsid().context("starting a session")?;
    let groups = groups
        .split(',')
        .filter(|g| !g.is_empty())
        .map(|g| g.parse().map(Gid::from_raw))
        .collect::<Result<Vec<_>, _>>()
        .context("reading the groups")?;
    setgroups(&groups).context("setting the groups")?;
    setgid(Gid::from_raw(gid)).context("setting the group")?;
    setuid(Uid::from_raw(uid)).context("setting the user")?;
    std::env::set_current_dir(cwd)
        .with_context(|| format!("changing to the working directory {cwd}"))?;
    let program_c = CString::new(program.as_str())?;
    let args = argv
        .iter()
        .map(|a| CString::new(a.as_str()))
        .collect::<Result<Vec<_>, _>>()?;
    let e = execvp(&program_c, &args).err();
    // As a shell says it: 127 for a program that isn't there, 126 for one
    // that can't run.
    eprintln!(
        "{program}: {}",
        e.map(|e| e.desc()).unwrap_or("can't run it")
    );
    std::process::exit(if e == Some(nix::errno::Errno::ENOENT) {
        127
    } else {
        126
    })
}
