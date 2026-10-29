//! `fctl-agent vm-serve`: runs what the host asks over the vsock, as
//! `docker exec` would, and keeps the VM's network linked to the host.

use super::users::{self, User};
use super::{VERSION, net};
use anyhow::{Context, Result};
use fctl_proto::vm::{Exec, MAX_LINE, PORT, Reply, Request, frame};
use nix::sys::signal::{Signal, kill, killpg};
use nix::time::{ClockId, clock_gettime, clock_settime};
use nix::unistd::Pid;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio_vsock::{VMADDR_CID_ANY, VsockAddr, VsockListener, VsockStream};

/// The PATH a program gets when the image sets none, as with Docker.
const DEFAULT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

pub async fn serve() -> Result<()> {
    tokio::spawn(net::run());
    let listener = VsockListener::bind(VsockAddr::new(VMADDR_CID_ANY, PORT))
        .context("listening on the vsock")?;
    loop {
        let (conn, _) = listener.accept().await.context("taking a connection")?;
        tokio::spawn(async move {
            if let Err(e) = handle(conn).await {
                tracing::debug!("a request from the host: {e:#}");
            }
        });
    }
}

/// Sets the VM's clock to the host's, `now_ms`, when they differ by more
/// than a second: the VM's runs on while the host sleeps.
pub fn follow_clock(now_ms: i64) {
    let Ok(here) = clock_gettime(ClockId::CLOCK_REALTIME) else {
        return;
    };
    let here_ms = here.tv_sec() * 1000 + here.tv_nsec() / 1_000_000;
    if (here_ms - now_ms).abs() <= 1000 {
        return;
    }
    let to = nix::sys::time::TimeSpec::new(now_ms / 1000, (now_ms % 1000) * 1_000_000);
    if let Err(e) = clock_settime(ClockId::CLOCK_REALTIME, to) {
        tracing::warn!("setting the clock: {e}");
    }
}

async fn reply(w: &mut (impl AsyncWrite + Unpin), r: &Reply) -> Result<()> {
    let mut line = serde_json::to_vec(r)?;
    line.push(b'\n');
    w.write_all(&line).await?;
    w.flush().await?;
    Ok(())
}

async fn handle(conn: VsockStream) -> Result<()> {
    let (r, mut w) = conn.into_split();
    let mut r = BufReader::new(r);
    let mut line = Vec::new();
    (&mut r)
        .take(MAX_LINE as u64)
        .read_until(b'\n', &mut line)
        .await?;
    let request = match serde_json::from_slice::<Request>(&line) {
        Ok(request) => request,
        Err(e) => {
            return reply(&mut w, &Reply::failed(format!("unknown request: {e}"))).await;
        }
    };
    match request {
        Request::Hello { now_ms } => {
            if let Some(t) = now_ms {
                follow_clock(t);
            }
            let hello = Reply {
                version: Some(VERSION.into()),
                ..Reply::default()
            };
            reply(&mut w, &hello).await
        }
        Request::Exec(exec) => run(exec, r, w).await,
        Request::Unpack { .. } => {
            reply(
                &mut w,
                &Reply::failed("this VM runs a sandbox; it unpacks no images"),
            )
            .await
        }
        Request::Shutdown => {
            reply(&mut w, &Reply::default()).await?;
            shutdown().await
        }
    }
}

/// The environment a program gets: the image's, with `HOME`, `HOSTNAME`
/// and `PATH` when it has none.
fn environment(image: &[String], user: &User, hostname: &str) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = image
        .iter()
        .filter_map(|kv| kv.split_once('='))
        .filter(|(k, _)| !k.is_empty())
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    for (k, v) in [
        ("PATH", DEFAULT_PATH),
        ("HOSTNAME", hostname),
        ("HOME", user.home.as_str()),
    ] {
        if !env.iter().any(|(have, _)| have == k) {
            env.push((k.to_owned(), v.to_owned()));
        }
    }
    env
}

/// Runs `exec`, and carries its input and output until it exits.
async fn run(
    exec: Exec,
    from_host: BufReader<tokio_vsock::OwnedReadHalf>,
    mut to_host: tokio_vsock::OwnedWriteHalf,
) -> Result<()> {
    if let Some(t) = exec.now_ms {
        follow_clock(t);
    }
    if exec.argv.is_empty() {
        return reply(&mut to_host, &Reply::failed("no program to run")).await;
    }
    let user = match users::find(exec.user.as_deref()) {
        Ok(user) => user,
        Err(e) => return reply(&mut to_host, &Reply::failed(e)).await,
    };
    let hostname = nix::unistd::gethostname()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cwd = exec
        .cwd
        .clone()
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "/".into());
    let groups: Vec<String> = user.groups.iter().map(u32::to_string).collect();
    let mut cmd = tokio::process::Command::new("/proc/self/exe");
    cmd.arg("vm-run")
        .arg(user.uid.to_string())
        .arg(user.gid.to_string())
        .arg(groups.join(","))
        .arg(&cwd)
        .arg("--")
        .args(&exec.argv)
        .env_clear()
        .envs(environment(&exec.env, &user, &hostname));
    if exec.detach {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        return match cmd.spawn() {
            Ok(mut child) => {
                // Reaped here; by PID 1 should vm-serve stop first.
                tokio::spawn(async move { child.wait().await });
                reply(&mut to_host, &Reply::default()).await
            }
            Err(e) => reply(&mut to_host, &Reply::failed(format!("starting it: {e}"))).await,
        };
    }
    cmd.stdin(if exec.stdin {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return reply(&mut to_host, &Reply::failed(format!("starting it: {e}"))).await,
    };
    reply(&mut to_host, &Reply::default()).await?;
    let pid = child.id().and_then(|p| i32::try_from(p).ok());
    let (frames, mut outgoing) = mpsc::channel::<(u8, Vec<u8>)>(16);
    let writer = tokio::spawn(async move {
        while let Some((kind, data)) = outgoing.recv().await {
            let header = frame::header(kind, data.len());
            if to_host.write_all(&header).await.is_err() || to_host.write_all(&data).await.is_err()
            {
                break;
            }
        }
        let _ = to_host.shutdown().await;
    });
    let mut outputs = Vec::new();
    for (kind, out) in [
        (
            frame::STDOUT,
            child
                .stdout
                .take()
                .map(|o| Box::new(o) as Box<dyn AsyncRead + Unpin + Send>),
        ),
        (
            frame::STDERR,
            child
                .stderr
                .take()
                .map(|o| Box::new(o) as Box<dyn AsyncRead + Unpin + Send>),
        ),
    ] {
        if let Some(out) = out {
            outputs.push(tokio::spawn(pump(out, kind, frames.clone())));
        }
    }
    // The host's input; when the host goes away, so does the program, with
    // everything it started.
    let stdin = child.stdin.take();
    let input = tokio::spawn(async move {
        take_input(from_host, stdin).await;
        if let Some(pid) = pid {
            let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
        }
    });
    let status = child.wait().await;
    // Output still under way, unless something it started holds the pipes.
    for out in outputs {
        let _ = tokio::time::timeout(Duration::from_secs(2), out).await;
    }
    let code = match status {
        Ok(s) => {
            use std::os::unix::process::ExitStatusExt;
            s.code().or(s.signal().map(|n| 128 + n)).unwrap_or(1)
        }
        Err(_) => 1,
    };
    let _ = frames
        .send((frame::EXIT, code.to_be_bytes().to_vec()))
        .await;
    drop(frames);
    let _ = writer.await;
    input.abort();
    Ok(())
}

/// Sends what `out` gives as frames of `kind`.
async fn pump(
    mut out: Box<dyn AsyncRead + Unpin + Send>,
    kind: u8,
    frames: mpsc::Sender<(u8, Vec<u8>)>,
) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match out.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if frames.send((kind, buf[..n].to_vec())).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// Writes the host's input to the program until the host ends it; returns
/// when the host goes away.
async fn take_input(
    mut from_host: BufReader<tokio_vsock::OwnedReadHalf>,
    mut stdin: Option<tokio::process::ChildStdin>,
) {
    let mut buf = vec![0u8; frame::MAX];
    loop {
        let mut header = [0u8; 5];
        if from_host.read_exact(&mut header).await.is_err() {
            return;
        }
        let Some((kind, len)) = frame::parse(header) else {
            return;
        };
        if from_host.read_exact(&mut buf[..len]).await.is_err() {
            return;
        }
        match kind {
            frame::STDIN => {
                if let Some(s) = stdin.as_mut()
                    && s.write_all(&buf[..len]).await.is_err()
                {
                    stdin = None;
                }
            }
            frame::STDIN_END => stdin = None,
            _ => {}
        }
    }
}

/// Stops every program, writes what the disks hold, and resets the VM,
/// which ends its Firecracker.
async fn shutdown() -> Result<()> {
    tracing::info!("shutting down");
    // Everything but PID 1 and this process.
    let _ = kill(Pid::from_raw(-1), Signal::SIGTERM);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let _ = kill(Pid::from_raw(-1), Signal::SIGKILL);
    super::boot::power_off()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_get_the_images_environment_and_a_home() {
        let user = User {
            uid: 1000,
            gid: 1000,
            groups: vec![1000],
            home: "/home/agent".into(),
        };
        let env = environment(
            &[
                "PATH=/opt/fctl/bin:/usr/bin".into(),
                "LANG=C.UTF-8".into(),
                "junk".into(),
            ],
            &user,
            "fctl-ses-1",
        );
        assert_eq!(
            env,
            [
                ("PATH".into(), "/opt/fctl/bin:/usr/bin".into()),
                ("LANG".into(), "C.UTF-8".into()),
                ("HOSTNAME".into(), "fctl-ses-1".into()),
                ("HOME".into(), "/home/agent".into()),
            ]
        );
        let bare = environment(&["HOME=/root".into()], &user, "h");
        assert!(bare.contains(&("PATH".into(), DEFAULT_PATH.into())));
        assert!(bare.contains(&("HOME".into(), "/root".into())));
    }
}
