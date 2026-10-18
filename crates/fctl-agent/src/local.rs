//! The session socket: programs in the sandbox reach the supervisor here.
//! `fctl edit` names and describes the session the agent runs in, and
//! `fctl publish` publishes screenshots for its pull requests.
//!
//! One request per connection, one JSON line each way (see
//! `fctl_proto::agent::SessionRequest`). The supervisor answers; this module
//! only carries the lines.

use anyhow::{Context, Result};
use fctl_proto::ApiError;
use fctl_proto::agent::{MAX_SESSION_REQUEST_BYTES, SessionReply, SessionRequest};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};

/// A request waiting for the supervisor's answer.
pub struct Call {
    pub request: SessionRequest,
    pub reply: oneshot::Sender<SessionReply>,
}

/// How long a client has to send its request (a screenshot is megabytes).
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the supervisor may take to answer: a screenshot waits for the
/// host to upload it, and the supervisor gives up on that sooner.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(150);

/// Beside the host's socket: `agent.sock` → `agent.session.sock`.
pub fn socket_path(host_socket: &Path) -> PathBuf {
    host_socket.with_extension("session.sock")
}

pub async fn accept_loop(listener: UnixListener, calls: mpsc::UnboundedSender<Call>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let calls = calls.clone();
        tokio::spawn(async move {
            if let Err(e) = serve(stream, calls).await {
                tracing::debug!("a session socket client: {e:#}");
            }
        });
    }
}

async fn serve(stream: UnixStream, calls: mpsc::UnboundedSender<Call>) -> Result<()> {
    let (read, mut write) = stream.into_split();
    let mut line = String::new();
    let mut reader = BufReader::new(read.take(MAX_SESSION_REQUEST_BYTES as u64));
    tokio::time::timeout(READ_TIMEOUT, reader.read_line(&mut line))
        .await
        .context("the client took too long to send its request")??;
    let reply = match serde_json::from_str::<SessionRequest>(line.trim()) {
        Ok(request) => {
            let (tx, rx) = oneshot::channel();
            let _ = calls.send(Call { request, reply: tx });
            match tokio::time::timeout(ANSWER_TIMEOUT, rx).await {
                Ok(Ok(reply)) => reply,
                Ok(Err(_)) => error("fctl-agent is shutting down"),
                Err(_) => error("fctl-agent did not answer in time"),
            }
        }
        Err(_) if line.len() >= MAX_SESSION_REQUEST_BYTES => error(&format!(
            "The request is larger than fctl-agent takes ({} MiB)",
            MAX_SESSION_REQUEST_BYTES / (1024 * 1024)
        )),
        Err(e) => error(&format!("fctl-agent did not understand the request: {e}")),
    };
    let mut text = serde_json::to_string(&reply)?;
    text.push('\n');
    write.write_all(text.as_bytes()).await?;
    write.shutdown().await?;
    Ok(())
}

fn error(message: &str) -> SessionReply {
    SessionReply::Error {
        error: ApiError::bad_request(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::SessionId;

    #[test]
    fn sits_beside_the_host_socket() {
        assert_eq!(
            socket_path(Path::new("/home/agent/.fctl/agent.sock")),
            Path::new("/home/agent/.fctl/agent.session.sock")
        );
        assert_eq!(
            socket_path(Path::new("/tmp/fctl-ada/0123456789abcdef.sock")),
            Path::new("/tmp/fctl-ada/0123456789abcdef.session.sock")
        );
    }

    #[tokio::test]
    async fn answers_one_request_per_connection() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("agent.session.sock");
        let listener = crate::link::bind(&path).await?;
        let (tx, mut rx) = mpsc::unbounded_channel::<Call>();
        tokio::spawn(accept_loop(listener, tx));
        // The supervisor's side: echo the title back.
        tokio::spawn(async move {
            while let Some(call) = rx.recv().await {
                let SessionRequest::Edit { title, .. } = call.request else {
                    continue;
                };
                let _ = call.reply.send(SessionReply::Edited {
                    id: SessionId("ses_1".into()),
                    project_slug: "widgets".into(),
                    number: 7,
                    title: title.unwrap_or_default(),
                    description: None,
                });
            }
        });
        let ask = |line: &'static str| {
            let path = path.clone();
            async move {
                let stream = UnixStream::connect(&path).await?;
                let (read, mut write) = stream.into_split();
                write.write_all(line.as_bytes()).await?;
                let mut reply = String::new();
                BufReader::new(read).read_line(&mut reply).await?;
                anyhow::Ok(serde_json::from_str::<SessionReply>(&reply)?)
            }
        };
        let edited = ask("{\"type\":\"edit\",\"title\":\"Fix the login test\"}\n").await?;
        assert!(
            matches!(edited, SessionReply::Edited { ref title, .. } if title == "Fix the login test")
        );
        let refused = ask("{\"type\":\"rename_everything\"}\n").await?;
        assert!(matches!(refused, SessionReply::Error { .. }));
        Ok(())
    }
}
