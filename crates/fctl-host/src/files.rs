//! Attachments on their way to the agent. The host fetches them from the
//! control plane with its runner token and hands them to fctl-agent as `File`
//! frames right before the `Configure` or `Deliver` that refers to them.
//!
//! One fetcher per session takes jobs in order and returns them in the same
//! order, so a message never overtakes an earlier one that is still
//! downloading. Files go out one at a time as they arrive.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fctl_client::Api;
use fctl_proto::SessionId;
use fctl_proto::agent::{AgentConfig, HostToAgent};
use fctl_proto::event::Attachment;
use fctl_proto::uplink::{Delivery, DeliveryKind};
use std::time::Duration;
use tokio::sync::mpsc;

/// Something to send to the agent once its files are there.
pub enum Job {
    Configure(Box<AgentConfig>),
    Deliver(Delivery),
}

impl Job {
    pub fn attachments(&self) -> &[Attachment] {
        match self {
            Self::Configure(c) => &c.spec.task_attachments,
            Self::Deliver(Delivery {
                kind: DeliveryKind::Message { attachments, .. },
                ..
            }) => attachments,
            Self::Deliver(_) => &[],
        }
    }
}

/// What the fetcher hands back, in order.
pub enum Ready {
    /// A `File` frame for the job that follows it.
    File(Box<HostToAgent>),
    /// An attachment that could not be fetched, and why.
    Failed(Attachment, String),
    /// The job itself, after its files.
    Job(Job),
}

/// Starts the fetcher for a session.
pub fn start(
    api: Api,
    session: SessionId,
) -> (mpsc::UnboundedSender<Job>, mpsc::UnboundedReceiver<Ready>) {
    let (job_tx, mut job_rx) = mpsc::unbounded_channel::<Job>();
    let (ready_tx, ready_rx) = mpsc::unbounded_channel::<Ready>();
    tokio::spawn(async move {
        while let Some(job) = job_rx.recv().await {
            for att in job.attachments() {
                let out = match fetch(&api, &session, att).await {
                    Ok(bytes) => Ready::File(Box::new(HostToAgent::File {
                        attachment: att.clone(),
                        data: STANDARD.encode(bytes),
                    })),
                    Err(e) => Ready::Failed(att.clone(), e),
                };
                if ready_tx.send(out).is_err() {
                    return;
                }
            }
            if ready_tx.send(Ready::Job(job)).is_err() {
                return;
            }
        }
    });
    (job_tx, ready_rx)
}

/// Downloads one attachment, retrying what may pass.
async fn fetch(api: &Api, session: &SessionId, att: &Attachment) -> Result<Vec<u8>, String> {
    let mut delay = Duration::from_secs(1);
    let mut attempt = 1;
    loop {
        let why = match api
            .runner_attachment(session.as_str(), att.id.as_str())
            .await
        {
            Ok(bytes) if bytes.len() as u64 == att.size => return Ok(bytes),
            Ok(bytes) => format!("got {} of {} bytes", bytes.len(), att.size),
            Err(e) if e.is_transient() => e.to_string(),
            Err(e) => return Err(e.to_string()),
        };
        if attempt == 4 {
            return Err(why);
        }
        tracing::warn!(
            "fetching {} for {session} (attempt {attempt}): {why}",
            att.id
        );
        tokio::time::sleep(delay).await;
        delay *= 3;
        attempt += 1;
    }
}
