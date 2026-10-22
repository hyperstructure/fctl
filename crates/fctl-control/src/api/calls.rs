//! What agents ask of the control plane about their session's project
//! ([`AgentCall`]): `fctl memory`, `fctl retro` and `fctl resolve` in a
//! sandbox go through fctl-agent to its host, which sends them here with
//! its runner token, for a session it runs. Each needs its project's
//! setting: `project_memory` for the memory, `retrospectives` for
//! retrospectives; resolving review threads needs an agent that may
//! comment on GitHub.

use crate::api::{memories, retrospectives, runners};
use crate::ctx::Ctx;
use crate::db::{q, s};
use crate::error::{AppResult, forbidden, not_found};
use crate::http;
use fctl_proto::api::{AgentAnswer, AgentCall, ReviewThread};
use fctl_proto::session::{GithubAccess, ProjectSettings, RepoAccess, RepoRef};
use serde::Deserialize;
use worker::{Request, Response};

/// The session a host asks for, and what its agents may do in its project.
pub struct RunnerSession {
    pub id: String,
    pub project_id: String,
    /// Who started it: the notes its agent writes name them.
    pub created_by: String,
    pub settings: ProjectSettings,
    /// Its project's workspace and primary repository.
    workspace_id: String,
    repo: Option<RepoRef>,
    /// What its agent may do on GitHub now.
    github_access: GithubAccess,
}

#[derive(Deserialize)]
struct Row {
    project_id: String,
    runner_id: Option<String>,
    created_by: String,
    settings: String,
    #[serde(default)]
    github_access: Option<String>,
    #[serde(default)]
    workspace_id: Option<String>,
    #[serde(default)]
    repo_owner: Option<String>,
    #[serde(default)]
    repo_name: Option<String>,
}

impl RunnerSession {
    /// The session, when it is assigned to the runner `runner_id` and that
    /// runner still serves its workspace.
    async fn load(ctx: &Ctx, runner_id: &str, session_id: &str) -> AppResult<Self> {
        let row: Option<Row> = q(
            &ctx.db,
            "SELECT s.project_id, s.runner_id, s.created_by, p.settings, s.github_access, \
             p.workspace_id, p.repo_owner, p.repo_name FROM sessions s \
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
        // Settings that don't read as settings turn both off, and leave
        // the agent reading only.
        let settings: ProjectSettings =
            serde_json::from_str(&row.settings).unwrap_or(ProjectSettings {
                project_memory: false,
                retrospectives: false,
                github_access: Some(GithubAccess::Read),
                ..Default::default()
            });
        let github_access = fctl_domain::limits::effective(
            row.github_access.as_deref().and_then(GithubAccess::parse),
            settings.github_access,
        );
        Ok(Self {
            id: session_id.to_owned(),
            project_id: row.project_id,
            created_by: row.created_by,
            settings,
            workspace_id: row.workspace_id.unwrap_or_default(),
            repo: match (row.repo_owner, row.repo_name) {
                (Some(owner), Some(name)) => Some(RepoRef { owner, name }),
                _ => None,
            },
            github_access,
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
        AgentCall::Remember {
            text,
            replaces,
            also_replaces,
        } => {
            session.require_memory()?;
            let replaces = fctl_domain::memory::replaced(replaces.as_ref(), &also_replaces)?;
            let writer = memories::Writer::Agent {
                session: &session.id,
                started_by: &session.created_by,
            };
            let (memory, replaced) = memories::remember(
                &ctx.db,
                &session.project_id,
                &writer,
                &text,
                &replaces,
                ctx.now,
            )
            .await?;
            // The first in `replaced`, as agents from before `also_replaced`
            // read it.
            let mut replaced = replaced.into_iter();
            AgentAnswer::Memory {
                memory,
                replaced: replaced.next(),
                also_replaced: replaced.collect(),
            }
        }
        AgentCall::MarkStale {
            id,
            reason,
            duplicate_of,
        } => {
            session.require_memory()?;
            AgentAnswer::Memory {
                memory: memories::mark_stale(
                    &ctx.db,
                    &session,
                    &id,
                    &reason,
                    duplicate_of.as_ref(),
                    ctx.now,
                )
                .await?,
                replaced: None,
                also_replaced: Vec::new(),
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
        AgentCall::Retrospectives => {
            session.require_retrospectives()?;
            AgentAnswer::Retrospectives {
                retrospectives: retrospectives::of_session(&ctx.db, &session.id).await?,
            }
        }
        AgentCall::UpdateRetrospective {
            id,
            outcome,
            worked_on,
            approach,
            challenges,
            insights,
        } => {
            session.require_retrospectives()?;
            let change = fctl_domain::retrospectives::Change {
                outcome,
                worked_on: worked_on.as_deref(),
                approach: approach.as_deref(),
                challenges: challenges.as_deref(),
                insights: insights.as_deref(),
            };
            AgentAnswer::Retrospective {
                retrospective: retrospectives::update(&ctx.db, &session, &id, &change, ctx.now)
                    .await?,
            }
        }
        AgentCall::ResolveThread { thread, unresolve } => AgentAnswer::Thread {
            thread: resolve_thread(ctx, &session, &thread, unresolve).await?,
        },
    };
    http::ok(&answer)
}

/// Resolves a review thread for an agent that may comment on GitHub, or
/// opens it again: one of a pull request in a repository the session may
/// write to, which its workspace may use. GitHub lets only tokens that can
/// push resolve threads, so the control plane does it with one of its own
/// (`github::Github::resolve_thread`).
async fn resolve_thread(
    ctx: &Ctx,
    session: &RunnerSession,
    thread: &str,
    unresolve: bool,
) -> AppResult<ReviewThread> {
    let thread = fctl_domain::threads::check_id(thread)?;
    if session.github_access < GithubAccess::Comment {
        return Err(forbidden(
            "This session's agent only reads on GitHub, so it can't resolve review threads.",
        ));
    }
    let grants = crate::spec::grants(&ctx.db, &session.project_id).await?;
    let Some(owner) = fctl_domain::repos::owner(session.repo.as_ref(), &grants) else {
        return Err(forbidden("This session's project has no repositories."));
    };
    let gh = crate::github::from_config(&ctx.cfg, ctx.now)?;
    let connections = crate::workspaces::connections(&ctx.db, &session.workspace_id).await?;
    let inst = crate::workspaces::installation(&ctx.db, &gh, &connections, owner).await?;
    // One answer for a thread that doesn't exist and one elsewhere, so that
    // asking tells nothing of the installation's other repositories.
    let elsewhere = || {
        forbidden(
            "This session has no review thread by that id: it resolves those of pull requests in the repositories it may write to.",
        )
    };
    let found = gh
        .review_thread(inst, &thread)
        .await?
        .ok_or_else(elsewhere)?;
    let writes = fctl_domain::repos::access(session.repo.as_ref(), &grants, &found.repo)
        == Some(RepoAccess::Write)
        && crate::workspaces::scope(&connections, &found.repo.owner)
            .is_some_and(|s| s.allows(&found.repo.name));
    if !writes {
        return Err(elsewhere());
    }
    let resolved = if found.resolved == !unresolve {
        found.resolved
    } else {
        gh.resolve_thread(inst, &found.repo, &thread, unresolve)
            .await?
    };
    Ok(ReviewThread {
        id: thread,
        resolved,
        pull_request_url: found.pull_request_url,
    })
}
