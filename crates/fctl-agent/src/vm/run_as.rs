//! `fctl-agent vm-run UID GID GROUPS CWD -- PROGRAM ARGS…`: what `vm-serve`
//! starts for each program the host runs. As root, it takes the user's
//! identity and a session of its own, so that the init can stop the program
//! with everything it started, then becomes the program.

use anyhow::{Context, Result, bail};
use nix::unistd::{Gid, Uid, execvp, setgid, setgroups, setsid, setuid};
use std::ffi::CString;

pub fn run_as(uid: u32, gid: u32, groups: &str, cwd: &str, argv: &[String]) -> Result<()> {
    let Some(program) = argv.first() else {
        bail!("no program to run");
    };
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
