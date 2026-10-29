//! Talking to a VM's init through Firecracker's vsock socket
//! ([`fctl_proto::vm`]).

use anyhow::{Context, Result, bail};
use fctl_proto::vm::{Exec, MAX_LINE, PORT, Reply, Request, frame};
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use std::time::{Duration, Instant};
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream,
    ReadBuf,
};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::oneshot;

pub type Reader = BufReader<OwnedReadHalf>;

/// How long the VM may take to take a request and answer it, what follows
/// aside: an init that takes longer is stuck, and nothing that waits for it
/// may hang with it.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Connects to `port` in the VM whose vsock socket is `socket`.
pub async fn connect(socket: &Path, port: u32) -> Result<(Reader, OwnedWriteHalf)> {
    let stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connecting to {}", socket.display()))?;
    let (r, mut w) = stream.into_split();
    w.write_all(format!("CONNECT {port}\n").as_bytes()).await?;
    let mut r = BufReader::new(r);
    let mut line = Vec::new();
    (&mut r).take(64).read_until(b'\n', &mut line).await?;
    if !line.starts_with(b"OK ") {
        bail!("nothing in the VM answers on port {port} yet");
    }
    Ok((r, w))
}

/// Reads one reply line.
pub async fn read_reply(r: &mut Reader) -> Result<Reply> {
    let mut line = Vec::new();
    r.take(MAX_LINE as u64).read_until(b'\n', &mut line).await?;
    if line.is_empty() {
        bail!("the VM closed the connection");
    }
    serde_json::from_slice(&line).context("reading the VM's reply")
}

/// Sends `request` to the init within [`REQUEST_TIMEOUT`]; returns the
/// connection and the init's first reply, or the error it says.
pub async fn request(socket: &Path, request: &Request) -> Result<(Reader, OwnedWriteHalf)> {
    let (r, w, reply) = ask(socket, request).await?;
    if let Some(e) = reply.error {
        bail!("{e}");
    }
    Ok((r, w))
}

/// Sends `request` and reads the first reply, within [`REQUEST_TIMEOUT`].
async fn ask(socket: &Path, request: &Request) -> Result<(Reader, OwnedWriteHalf, Reply)> {
    let asking = async {
        let (mut r, mut w) = connect(socket, PORT).await?;
        let mut line = serde_json::to_vec(request)?;
        line.push(b'\n');
        w.write_all(&line).await?;
        let reply = read_reply(&mut r).await?;
        Ok::<_, anyhow::Error>((r, w, reply))
    };
    tokio::time::timeout(REQUEST_TIMEOUT, asking)
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "the VM didn't answer within {} seconds",
                REQUEST_TIMEOUT.as_secs()
            )
        })?
}

/// The host's clock, which VMs follow.
pub fn now_ms() -> i64 {
    crate::expiry::now_ms()
}

/// The init's version, when it answers.
pub async fn hello(socket: &Path) -> Result<String> {
    let hello = Request::Hello {
        now_ms: Some(now_ms()),
    };
    let (_, _, reply) = ask(socket, &hello).await?;
    Ok(reply.version.unwrap_or_default())
}

/// Waits for the init to answer, for at most `within`; `alive` says whether
/// the VM still runs.
pub async fn wait_ready(
    socket: &Path,
    within: Duration,
    alive: impl Fn() -> bool,
) -> Result<String> {
    let until = Instant::now() + within;
    loop {
        let last = match tokio::time::timeout(Duration::from_secs(5), hello(socket)).await {
            Ok(Ok(version)) => return Ok(version),
            Ok(Err(e)) => e,
            Err(_) => anyhow::anyhow!("no answer within 5 seconds"),
        };
        if !alive() {
            bail!("the VM stopped while it booted");
        }
        if Instant::now() > until {
            bail!(
                "the VM did not answer within {} seconds: {last:#}",
                within.as_secs()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn send_frame(w: &mut OwnedWriteHalf, kind: u8, data: &[u8]) -> std::io::Result<()> {
    w.write_all(&frame::header(kind, data.len())).await?;
    w.write_all(data).await
}

/// What a program wrote, and how it exited.
#[derive(Debug, Default)]
pub struct Output {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// The most output [`run`] keeps of a program.
const MAX_OUTPUT: usize = 64 * 1024 * 1024;

/// Runs `exec` with `input`, to its end.
pub async fn run(socket: &Path, mut exec: Exec, input: Option<&[u8]>) -> Result<Output> {
    exec.stdin = input.is_some();
    exec.now_ms = Some(now_ms());
    let (mut r, mut w) = request(socket, &Request::Exec(exec)).await?;
    let feed = async {
        if let Some(input) = input {
            for chunk in input.chunks(frame::MAX) {
                send_frame(&mut w, frame::STDIN, chunk).await?;
            }
            send_frame(&mut w, frame::STDIN_END, &[]).await?;
        }
        // The write half stays open: the init stops a program whose host
        // went away.
        std::future::pending::<std::io::Result<()>>().await
    };
    let read = async {
        let mut out = Output::default();
        let mut buf = vec![0u8; frame::MAX];
        loop {
            let mut header = [0u8; 5];
            r.read_exact(&mut header)
                .await
                .context("the VM closed the connection before the program exited")?;
            let (kind, len) = frame::parse(header).context("a frame too long")?;
            r.read_exact(&mut buf[..len]).await?;
            let data = &buf[..len];
            match kind {
                frame::STDOUT if out.stdout.len() + len <= MAX_OUTPUT => {
                    out.stdout.extend_from_slice(data);
                }
                frame::STDERR if out.stderr.len() + len <= MAX_OUTPUT => {
                    out.stderr.extend_from_slice(data);
                }
                frame::EXIT => {
                    out.code = data
                        .get(..4)
                        .and_then(|b| b.try_into().ok())
                        .map(i32::from_be_bytes)
                        .unwrap_or(1);
                    return Ok::<Output, anyhow::Error>(out);
                }
                _ => {}
            }
        }
    };
    tokio::select! {
        out = read => out,
        e = feed => Err(e.err().map(anyhow::Error::from).unwrap_or_else(|| anyhow::anyhow!("closed"))),
    }
}

/// Starts `exec` without waiting for it, with nothing in or out.
pub async fn detach(socket: &Path, mut exec: Exec) -> Result<()> {
    exec.detach = true;
    exec.stdin = false;
    exec.now_ms = Some(now_ms());
    request(socket, &Request::Exec(exec)).await.map(drop)
}

/// The tasks that carry a program's input and output, stopped when nothing
/// holds the program's pipes any more: the connection closes, and the init
/// stops the program.
pub struct Tasks(Vec<tokio::task::JoinHandle<()>>);

impl Drop for Tasks {
    fn drop(&mut self) {
        for t in &self.0 {
            t.abort();
        }
    }
}

/// A pipe of a program in a VM, which keeps its tasks running.
pub struct Pipe<T> {
    inner: T,
    _tasks: Arc<Tasks>,
}

impl<T: AsyncRead + Unpin> AsyncRead for Pipe<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Pipe<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// A program in a VM, with its input and output as pipes.
pub struct Started {
    pub stdin: Pipe<DuplexStream>,
    pub stdout: Pipe<DuplexStream>,
    pub stderr: Pipe<DuplexStream>,
    /// Its exit code, or `None` when the connection closed first.
    pub exit: oneshot::Receiver<Option<i32>>,
}

/// Starts `exec` with its input and output piped.
pub async fn start(socket: &Path, mut exec: Exec) -> Result<Started> {
    exec.stdin = true;
    exec.detach = false;
    exec.now_ms = Some(now_ms());
    let (mut from_vm, mut to_vm) = request(socket, &Request::Exec(exec)).await?;
    let (stdin, mut stdin_rx) = tokio::io::duplex(256 * 1024);
    let (mut stdout_tx, stdout) = tokio::io::duplex(256 * 1024);
    let (mut stderr_tx, stderr) = tokio::io::duplex(64 * 1024);
    let (exit_tx, exit) = oneshot::channel();
    let output = tokio::spawn(async move {
        let mut buf = vec![0u8; frame::MAX];
        let mut code = None;
        loop {
            let mut header = [0u8; 5];
            if from_vm.read_exact(&mut header).await.is_err() {
                break;
            }
            let Some((kind, len)) = frame::parse(header) else {
                break;
            };
            if from_vm.read_exact(&mut buf[..len]).await.is_err() {
                break;
            }
            let data = &buf[..len];
            match kind {
                // A reader that went away takes nothing more.
                frame::STDOUT => {
                    let _ = stdout_tx.write_all(data).await;
                }
                frame::STDERR => {
                    let _ = stderr_tx.write_all(data).await;
                }
                frame::EXIT => {
                    code = data
                        .get(..4)
                        .and_then(|b| b.try_into().ok())
                        .map(i32::from_be_bytes);
                    break;
                }
                _ => {}
            }
        }
        let _ = exit_tx.send(code);
    });
    let input = tokio::spawn(async move {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match stdin_rx.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if send_frame(&mut to_vm, frame::STDIN, &buf[..n])
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
        let _ = send_frame(&mut to_vm, frame::STDIN_END, &[]).await;
        // Holds the connection open until the pipes go.
        std::future::pending::<()>().await;
    });
    let tasks = Arc::new(Tasks(vec![output, input]));
    Ok(Started {
        stdin: Pipe {
            inner: stdin,
            _tasks: tasks.clone(),
        },
        stdout: Pipe {
            inner: stdout,
            _tasks: tasks.clone(),
        },
        stderr: Pipe {
            inner: stderr,
            _tasks: tasks,
        },
        exit,
    })
}

/// Asks the init to stop the VM.
pub async fn shutdown(socket: &Path) -> Result<()> {
    request(socket, &Request::Shutdown).await.map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;

    /// A stand-in for Firecracker's socket and the init behind it: takes
    /// `CONNECT`, then answers each request as `answer` does.
    async fn fake_vm<F, Fut>(dir: &Path, answer: F) -> Result<std::path::PathBuf>
    where
        F: Fn(Request, Reader, OwnedWriteHalf) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let socket = dir.join("v.sock");
        let listener = UnixListener::bind(&socket)?;
        let answer = Arc::new(answer);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let answer = answer.clone();
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut r = BufReader::new(r);
                    let mut line = String::new();
                    if r.read_line(&mut line).await.is_err() || line != format!("CONNECT {PORT}\n")
                    {
                        return;
                    }
                    let _ = w.write_all(b"OK 1073741824\n").await;
                    line.clear();
                    if r.read_line(&mut line).await.is_err() {
                        return;
                    }
                    if let Ok(request) = serde_json::from_str::<Request>(&line) {
                        answer(request, r, w).await;
                    }
                });
            }
        });
        Ok(socket)
    }

    async fn reply(w: &mut OwnedWriteHalf, r: &Reply) {
        let mut line = serde_json::to_vec(r).unwrap_or_default();
        line.push(b'\n');
        let _ = w.write_all(&line).await;
    }

    #[tokio::test]
    async fn runs_programs_and_carries_their_output() -> Result<()> {
        let dir = tempfile::tempdir()?;
        // Echoes its input to stdout, says something on stderr, exits 3.
        let socket = fake_vm(dir.path(), |request, mut r, mut w| async move {
            let Request::Exec(exec) = request else {
                return reply(&mut w, &Reply::failed("not exec")).await;
            };
            if exec.argv.first().map(String::as_str) == Some("missing") {
                return reply(&mut w, &Reply::failed("starting it: not found")).await;
            }
            reply(&mut w, &Reply::default()).await;
            let mut input = Vec::new();
            loop {
                let mut header = [0u8; 5];
                if r.read_exact(&mut header).await.is_err() {
                    return;
                }
                let Some((kind, len)) = frame::parse(header) else {
                    return;
                };
                let mut data = vec![0u8; len];
                if r.read_exact(&mut data).await.is_err() {
                    return;
                }
                match kind {
                    frame::STDIN => input.extend(data),
                    _ => break,
                }
            }
            let _ = send_frame(&mut w, frame::STDOUT, &input).await;
            let _ = send_frame(&mut w, frame::STDERR, b"warned").await;
            let _ = send_frame(&mut w, frame::EXIT, &3i32.to_be_bytes()).await;
        })
        .await?;
        let exec = Exec {
            argv: vec!["cat".into()],
            ..Exec::default()
        };
        let out = run(&socket, exec.clone(), Some(b"hello")).await?;
        assert_eq!(out.code, 3);
        assert_eq!(out.stdout, b"hello");
        assert_eq!(out.stderr, b"warned");
        let missing = Exec {
            argv: vec!["missing".into()],
            ..Exec::default()
        };
        let e = run(&socket, missing, None)
            .await
            .err()
            .map(|e| e.to_string());
        assert_eq!(e.as_deref(), Some("starting it: not found"));
        // Started: input through its pipe, output through the others.
        let mut started = start(&socket, exec).await?;
        started.stdin.write_all(b"piped").await?;
        started.stdin.shutdown().await?;
        drop(started.stdin);
        let mut stdout = String::new();
        started.stdout.read_to_string(&mut stdout).await?;
        assert_eq!(stdout, "piped");
        assert_eq!(started.exit.await.ok().flatten(), Some(3));
        Ok(())
    }

    #[tokio::test]
    async fn waits_for_the_init_and_says_when_nothing_answers() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let gone = dir.path().join("none.sock");
        let e = wait_ready(&gone, Duration::from_millis(300), || true).await;
        assert!(e.is_err());
        let stopped = wait_ready(&gone, Duration::from_secs(30), || false).await;
        assert_eq!(
            stopped.err().map(|e| e.to_string()).as_deref(),
            Some("the VM stopped while it booted")
        );
        let socket = fake_vm(dir.path(), |request, _, mut w| async move {
            if matches!(request, Request::Hello { now_ms: Some(_) }) {
                reply(
                    &mut w,
                    &Reply {
                        version: Some("0.27.0".into()),
                        ..Reply::default()
                    },
                )
                .await;
            }
        })
        .await?;
        assert_eq!(
            wait_ready(&socket, Duration::from_secs(5), || true).await?,
            "0.27.0"
        );
        Ok(())
    }
}
