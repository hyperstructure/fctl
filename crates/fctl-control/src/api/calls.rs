//! What agents ask of the control plane about their session's project
//! ([`AgentCall`]): `fctl memory` and `fctl retro` in a sandbox go through
//! fctl-agent to its host, which sends them here with its runner token, for
//! a session it runs. Each needs its project's setting: `project_memory`
//! for the memory, `retrospectives` for retrospectives.

use crate::api::{memories, retrospectives, runners};
use crate::ctx::Ctx;
use crate::db::{q, s};
use crate::error::{AppResult, forbidden, not_found};
use crate::http;
use fctl_proto::api::{AgentAnswer, AgentCall};
use fctl_proto::session::ProjectSettings;
use serde::Deserialize;
use worker::{Request, Response};

/// The session a host asks for, and what its agents may do in its project.
pub struct RunnerSession {
    pub id: String,
    pub project_id: String,
    /// Who started it: the notes its agent writes name them.
    pub created_by: String,
    pub settings: ProjectSettings,
}

#[derive(Deserialize)]
struct Row {
    project_id: String,
    runner_id: Option<String>,
    created_by: String,
    settings: String,
}

impl RunnerSession {
    /// The session, when it is assigned to the runner `runner_id` and that
    /// runner still serves its workspace.
    async fn load(ctx: &Ctx, runner_id: &str, session_id: &str) -> AppResult<Self> {
        let row: Option<Row> = q(
            &ctx.db,
            "SELECT s.project_id, s.runner_id, s.created_by, p.settings FROM sessions s \
             JOIN projects p ON p.id = s.project_id WHERE s.id = ?",
            &[s(session_id)],
        )?
        .first()
        .await?;
        let row = row.ok_or_else(|| not_found("Session"))?;
        if row.runner_id.as_deref() != Some(runner_id) {
            return Err(forbidden("This session is not assigned to this runner"));
        }
        if !crate::spec::runner_serves(&ctx.db, runner_id, session_id).await? {
            return Err(crate::spec::not_served());
        }
        Ok(Self {
            id: session_id.to_owned(),
            project_id: row.project_id,
            created_by: row.created_by,
            settings: serde_json::from_str(&row.settings).unwrap_or_default(),
        })
    }

    fn require_memory(&self) -> AppResult<()> {
        if self.settings.project_memory {
            Ok(())
        } else {
            Err(forbidden(
                "This project keeps no memory. Its maintainers can turn it on in the project's settings.",
            ))
        }
    }

    fn require_retrospectives(&self) -> AppResult<()> {
        if self.settings.retrospectives {
            Ok(())
        } else {
            Err(forbidden(
                "This project doesn't ask for retrospectives. Its maintainers can turn them on in the project's settings.",
            ))
        }
    }
}

/// `POST /api/v1/runner/sessions/{id}/calls` with an [`AgentCall`], from
/// the host the session runs on. Answers with an [`AgentAnswer`].
pub async fn runner_call(
    ctx: &mut Ctx,
    req: &mut Request,
    session_id: &str,
) -> AppResult<Response> {
    let r = runners::runner_auth(ctx, req).await?;
    let call: AgentCall = http::body(req).await?;
    let session = RunnerSession::load(ctx, &r.id, session_id).await?;
    let answer = match call {
        AgentCall::Memories { stale } => {
            session.require_memory()?;
            AgentAnswer::Memories {
                memories: memories::list(&ctx.db, &session.project_id, stale).await?,
            }
        }
        AgentCall::Remember { text, replaces } => {
            session.require_memory()?;
            let (memory, replaced) =
                memories::remember(&ctx.db, &session, &text, replaces.as_ref(), ctx.now).await?;
            AgentAnswer::Memory { memory, replaced }
        }
        AgentCall::MarkStale { id, reason } => {
            session.require_memory()?;
            AgentAnswer::Memory {
                memory: memories::mark_stale(&ctx.db, &session, &id, &reason, ctx.now).await?,
                replaced: None,
            }
        }
        AgentCall::Retrospective {
            outcome,
            worked_on,
            approach,
            challenges,
            insights,
        } => {
            session.require_retrospectives()?;
            let parts = fctl_domain::retrospectives::check(
                &worked_on,
                approach.as_deref(),
                challenges.as_deref(),
                insights.as_deref(),
            )?;
            AgentAnswer::Retrospective {
                retrospective: retrospectives::record(&ctx.db, &session, outcome, parts, ctx.now)
                    .await?,
            }
        }
    };
    http::ok(&answer)
}
