//! The session socket: programs in the sandbox reach the supervisor here.
//! `fctl edit` names and describes the session the agent runs in.
//!
//! One request per connection, one JSON line each way (see
//! `fctl_proto::agent::SessionRequest`). The supervisor answers; this module
//! only carries the lines.

use anyhow::Result;
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
            if tokio::time::timeout(Duration::from_secs(10), serve(stream, calls))
                .await
                .is_err()
            {
                tracing::debug!("a session socket client took too long");
            }
        });
    }
}

async fn serve(stream: UnixStream, calls: mpsc::UnboundedSender<Call>) -> Result<()> {
    let (read, mut write) = stream.into_split();
    let mut line = String::new();
    BufReader::new(read.take(MAX_SESSION_REQUEST_BYTES as u64))
        .read_line(&mut line)
        .await?;
    let reply = match serde_json::from_str::<SessionRequest>(line.trim()) {
        Ok(request) => {
            let (tx, rx) = oneshot::channel();
            let _ = calls.send(Call { request, reply: tx });
            rx.await
                .unwrap_or_else(|_| error("fctl-agent is shutting down"))
        }
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
                let SessionRequest::Edit { title, .. } = call.request;
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
