//! Limited access tokens, and what sessions' agents may do on GitHub
//! (docs/architecture.md#access-tokens).
//!
//! A personal access token may be limited ([`Limits`]): in what it does
//! (reading, starting and steering sessions, or everything its person may),
//! where (some workspaces and projects), and in what the agents of its
//! sessions may do on GitHub. A limited token reaches projects and sessions
//! only ([`allows`]), never the account, workspaces' people and settings,
//! other tokens or administration, so that one handed to CI or a bot can't
//! make itself a token without limits.
//!
//! A session's agent pushes, comments or only reads on GitHub
//! ([`GithubAccess`]): as much as its project and the token that starts it
//! allow, or less when it asks ([`session_github`]). Its GitHub tokens
//! carry that ([`crate::repos::permissions`]), so GitHub enforces it.

use crate::perm::{self, SessionAccess, lower_project_role, lower_session_role};
use crate::validate::Result;
use fctl_proto::ApiError;
use fctl_proto::api::TokenAccess;
use fctl_proto::session::{GithubAccess, ProjectRole, SessionRole};
use serde::{Deserialize, Serialize};

/// Most workspaces and projects one token names.
pub const MAX_PLACES: usize = 50;

/// What a limited access token may do, as D1 keeps it (`tokens.scope`). A
/// field this version doesn't know, from a newer one, makes it unreadable,
/// so that it reaches nothing rather than ignore a limit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub access: TokenAccess,
    /// Where it reaches: `None`, every project its person sees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub places: Option<Places>,
    /// The most the agents of its sessions may do on GitHub: `None`, push.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_access: Option<GithubAccess>,
}

/// The workspaces whose every project a token reaches, and the projects it
/// reaches besides, by id. Once all of them are gone it reaches nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Places {
    #[serde(default)]
    pub workspaces: Vec<String>,
    #[serde(default)]
    pub projects: Vec<String>,
}

/// Checks the limits a new token asks for. One that may do everything in
/// its projects may change their settings, and so what every agent there
/// does (their setup scripts, instructions, images and secrets), so a
/// limit on what its sessions' agents may do on GitHub would hold nothing:
/// that takes a token that only runs sessions.
pub fn check(access: TokenAccess, github_access: Option<GithubAccess>) -> Result<()> {
    if access == TokenAccess::Full && github_access.is_some_and(|g| g != GithubAccess::Push) {
        return Err(ApiError::validation(
            "github_access",
            "A token that does everything in its projects can change their settings, and so what every agent there does. To hold its agents to commenting or reading, let it run sessions instead.",
        ));
    }
    Ok(())
}

impl Limits {
    /// The limits of a new token, `None` for one without any.
    pub fn new(
        access: TokenAccess,
        places: Option<Places>,
        github_access: Option<GithubAccess>,
    ) -> Option<Self> {
        let places = places.filter(|p| !p.workspaces.is_empty() || !p.projects.is_empty());
        let github_access = github_access.filter(|g| *g != GithubAccess::Push);
        (access != TokenAccess::Full || places.is_some() || github_access.is_some()).then_some(
            Self {
                access,
                places,
                github_access,
            },
        )
    }

    /// The limits D1 keeps for a token (`tokens.scope`): `None` for a token
    /// without any. Limits that don't read as limits reach nothing, rather
    /// than everything.
    pub fn parse(stored: Option<&str>) -> Option<Self> {
        let stored = stored.filter(|s| !s.trim().is_empty())?;
        Some(serde_json::from_str(stored).unwrap_or(Self {
            access: TokenAccess::Read,
            places: Some(Places::default()),
            github_access: Some(GithubAccess::Read),
        }))
    }

    /// As D1 keeps them. Limits that didn't serialize would be kept as
    /// ones that reach nothing ([`Self::parse`]), never as none.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned())
    }

    /// Whether it reaches the project `project_id` of workspace
    /// `workspace_id`.
    pub fn reaches(&self, workspace_id: &str, project_id: &str) -> bool {
        self.places.as_ref().is_none_or(|p| {
            p.workspaces.iter().any(|w| w == workspace_id)
                || p.projects.iter().any(|id| id == project_id)
        })
    }

    /// Whether it reaches some project of workspace `workspace_id`, among
    /// `projects` (the workspace's projects it names, by workspace).
    pub fn reaches_workspace(&self, workspace_id: &str, project_workspaces: &[String]) -> bool {
        self.places.as_ref().is_none_or(|p| {
            p.workspaces.iter().any(|w| w == workspace_id)
                || project_workspaces.iter().any(|w| w == workspace_id)
        })
    }

    /// The most it does in a project: a viewer's, an editor's, or all its
    /// person may.
    pub fn project_cap(&self) -> ProjectRole {
        match self.access {
            TokenAccess::Read => ProjectRole::Viewer,
            TokenAccess::Run => ProjectRole::Editor,
            TokenAccess::Full => ProjectRole::Owner,
        }
    }

    /// The role it has in a project where its person has `role`.
    pub fn project_role(
        &self,
        workspace_id: &str,
        project_id: &str,
        role: Option<ProjectRole>,
    ) -> Option<ProjectRole> {
        if !self.reaches(workspace_id, project_id) {
            return None;
        }
        role.map(|r| lower_project_role(r, self.project_cap()))
    }

    /// The role it has in a session, whose person's relation to it is
    /// `access` and whose agent started able to do `github` on GitHub:
    /// their role, with their role in its project counting for at most
    /// [`Self::project_cap`], as a viewer when the token only reads, and as
    /// a viewer too when the agent started able to do more on GitHub than
    /// the token allows its agents. So a token for review agents can't
    /// steer an agent that pushes, also once its project holds it lower,
    /// while its sandbox still has the tokens it started with.
    pub fn session_role(
        &self,
        workspace_id: &str,
        project_id: &str,
        access: SessionAccess,
        github: GithubAccess,
    ) -> Option<SessionRole> {
        if !self.reaches(workspace_id, project_id) {
            return None;
        }
        let role = perm::session_role_within(access, self.project_cap())?;
        let read_only = self.access == TokenAccess::Read
            || github > self.github_access.unwrap_or(GithubAccess::Push);
        Some(if read_only {
            lower_session_role(role, SessionRole::Viewer)
        } else {
            role
        })
    }
}

/// What a limited token at `access` may call: `method` on the API path
/// after `/api/v1/` (`segs`). Every one reads its projects and sessions;
/// one that runs sessions also starts and steers them and keeps projects'
/// memory; one that may do everything also manages its projects. None
/// reaches anything else. Roles still apply: this only says what it may
/// try.
pub fn allows(access: TokenAccess, method: &str, segs: &[&str]) -> Result<()> {
    use TokenAccess::{Full, Read, Run};
    let needs = match (method, segs) {
        // Who it belongs to, and signing itself out.
        ("GET", ["auth", "me"]) | ("POST", ["auth", "logout"]) => Read,
        // Open to anyone anyway.
        ("GET", ["instance" | "health"]) | ("GET" | "HEAD", ["published", ..]) => Read,
        ("GET", ["workspaces"]) => Read,
        (
            "GET",
            ["projects"]
            | ["projects", _]
            | [
                "projects",
                _,
                "repos" | "members" | "memories" | "retrospectives" | "review-agents" | "reviews",
            ]
            | ["memories", _],
        ) => Read,
        (
            "GET",
            ["sessions"]
            | ["sessions", _]
            | ["sessions", _, "events" | "retrospectives" | "ws" | "shares"]
            | ["sessions", _, "attachments", _],
        ) => Read,
        // Starting and steering sessions (a review's too), and keeping the memory.
        (
            "POST",
            [
                "projects",
                _,
                "sessions" | "attachments" | "memories" | "reviews",
            ],
        ) => Run,
        (
            "POST",
            [
                "sessions",
                _,
                "messages" | "interrupt" | "stop" | "resume" | "attachments",
            ],
        ) => Run,
        ("PATCH", ["sessions", _] | ["memories", _]) => Run,
        ("PUT" | "DELETE", ["sessions", _, "follow"]) => Run,
        // Managing its projects, and deleting.
        ("PATCH" | "DELETE", ["projects", _]) => Full,
        ("PATCH", ["projects", _, "review-agents"]) => Full,
        ("GET", ["projects", _, "secrets"]) => Full,
        ("PUT" | "DELETE", ["projects", _, "secrets", _]) => Full,
        ("POST", ["projects", _, "members" | "repos"]) => Full,
        ("PATCH" | "DELETE", ["projects", _, "members", _]) => Full,
        ("DELETE", ["projects", _, "repos", _, _]) => Full,
        ("POST", ["sessions", _, "shares"]) | ("DELETE", ["sessions", _, "shares", _]) => Full,
        ("DELETE", ["memories", _] | ["retrospectives", _] | ["published", ..]) => Full,
        _ => return Err(refused(access)),
    };
    if access >= needs {
        Ok(())
    } else {
        Err(refused(access))
    }
}

fn refused(access: TokenAccess) -> ApiError {
    let what = match access {
        TokenAccess::Read => "it only reads projects and sessions",
        TokenAccess::Run => "it only reads, starts and steers sessions",
        TokenAccess::Full => "it only reaches projects and their sessions",
    };
    ApiError::forbidden(format!(
        "This access token can't do that: {what}. A token without limits can (Settings → Access tokens, or fctl token)."
    ))
}

/// What a new session's agent may do on GitHub: what it asks for, which may
/// be at most what its project (`project`, its settings' `github_access`)
/// and the token starting it (`token`) allow; what they allow when it asks
/// for nothing.
pub fn session_github(
    asked: Option<GithubAccess>,
    project: Option<GithubAccess>,
    token: Option<GithubAccess>,
) -> Result<GithubAccess> {
    let project = project.unwrap_or_default();
    let token = token.unwrap_or_default();
    let most = project.min(token);
    let Some(asked) = asked else {
        return Ok(most);
    };
    if asked <= most {
        return Ok(asked);
    }
    let (whose, most) = if project < asked {
        ("This project's agents", project)
    } else {
        ("The agents of this access token's sessions", token)
    };
    Err(ApiError::forbidden(format!(
        "{whose} may at most {} on GitHub.",
        verb(most)
    )))
}

/// What a session's agent may do on GitHub now: what it started with
/// (`None`: push, from before sessions had one), at most what its project
/// allows now.
pub fn effective(session: Option<GithubAccess>, project: Option<GithubAccess>) -> GithubAccess {
    session.unwrap_or_default().min(project.unwrap_or_default())
}

/// What an agent at `access` does, for messages: "push", "comment", "read".
pub fn verb(access: GithubAccess) -> &'static str {
    access.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Membership;
    use fctl_proto::api::BaseRole;
    use fctl_proto::session::WorkspaceRole;

    fn limits(access: TokenAccess) -> Limits {
        Limits {
            access,
            places: None,
            github_access: None,
        }
    }

    fn admin() -> SessionAccess {
        SessionAccess {
            membership: Some(Membership {
                role: WorkspaceRole::Admin,
                base_role: BaseRole::None,
                blocked: false,
            }),
            project_role: None,
            share_role: None,
            is_creator: false,
        }
    }

    #[test]
    fn tokens_without_limits_keep_none() {
        assert_eq!(Limits::new(TokenAccess::Full, None, None), None);
        assert_eq!(
            Limits::new(
                TokenAccess::Full,
                Some(Places::default()),
                Some(GithubAccess::Push)
            ),
            None
        );
        let l = Limits::new(TokenAccess::Run, None, Some(GithubAccess::Push));
        assert_eq!(l, Some(limits(TokenAccess::Run)));
    }

    #[test]
    fn stored_limits_round_trip_and_fail_closed() {
        let l = Limits {
            access: TokenAccess::Run,
            places: Some(Places {
                workspaces: vec!["wsp_1".into()],
                projects: vec!["prj_2".into()],
            }),
            github_access: Some(GithubAccess::Comment),
        };
        assert_eq!(Limits::parse(Some(&l.to_json())), Some(l));
        assert_eq!(Limits::parse(None), None);
        assert_eq!(Limits::parse(Some("")), None);
        let broken =
            Limits::parse(Some(r#"{"access":"admin"}"#)).unwrap_or(limits(TokenAccess::Full));
        assert_eq!(broken.access, TokenAccess::Read);
        assert!(!broken.reaches("wsp_1", "prj_1"));
        // A limit from a newer version this one doesn't know.
        let newer = Limits::parse(Some(r#"{"access":"run","hours":"9-17"}"#))
            .unwrap_or(limits(TokenAccess::Full));
        assert_eq!(newer.access, TokenAccess::Read);
        assert!(!newer.reaches("wsp_1", "prj_1"));
        // What `to_json` keeps when serializing fails reaches nothing too.
        let fallback = Limits::parse(Some("{}")).unwrap_or(limits(TokenAccess::Full));
        assert_eq!(fallback.access, TokenAccess::Read);
        assert!(!fallback.reaches("wsp_1", "prj_1"));
    }

    #[test]
    fn a_token_that_does_everything_holds_its_agents_to_nothing() {
        use GithubAccess::{Comment, Push, Read};
        assert!(check(TokenAccess::Full, Some(Comment)).is_err());
        assert!(check(TokenAccess::Full, Some(Read)).is_err());
        assert!(check(TokenAccess::Full, Some(Push)).is_ok());
        assert!(check(TokenAccess::Full, None).is_ok());
        assert!(check(TokenAccess::Run, Some(Comment)).is_ok());
        assert!(check(TokenAccess::Read, Some(Read)).is_ok());
    }

    #[test]
    fn places_limit_the_projects_it_reaches() {
        let mut l = limits(TokenAccess::Full);
        assert!(l.reaches("wsp_1", "prj_1"));
        l.places = Some(Places {
            workspaces: vec!["wsp_1".into()],
            projects: vec!["prj_9".into()],
        });
        assert!(l.reaches("wsp_1", "prj_1"));
        assert!(l.reaches("wsp_2", "prj_9"));
        assert!(!l.reaches("wsp_2", "prj_2"));
        assert!(l.reaches_workspace("wsp_2", &["wsp_2".into()]));
        assert!(!l.reaches_workspace("wsp_3", &["wsp_2".into()]));
        assert_eq!(
            l.project_role("wsp_2", "prj_2", Some(ProjectRole::Owner)),
            None
        );
        // Gone places leave it reaching nothing.
        l.places = Some(Places::default());
        assert!(!l.reaches("wsp_1", "prj_1"));
    }

    #[test]
    fn access_caps_project_roles() {
        let owner = Some(ProjectRole::Owner);
        assert_eq!(
            limits(TokenAccess::Read).project_role("w", "p", owner),
            Some(ProjectRole::Viewer)
        );
        assert_eq!(
            limits(TokenAccess::Run).project_role("w", "p", owner),
            Some(ProjectRole::Editor)
        );
        assert_eq!(
            limits(TokenAccess::Full).project_role("w", "p", owner),
            owner
        );
        // Never more than the person's own.
        assert_eq!(
            limits(TokenAccess::Run).project_role("w", "p", Some(ProjectRole::Viewer)),
            Some(ProjectRole::Viewer)
        );
        assert_eq!(limits(TokenAccess::Full).project_role("w", "p", None), None);
    }

    #[test]
    fn access_caps_session_roles() {
        let push = GithubAccess::Push;
        let theirs = SessionAccess {
            is_creator: true,
            ..admin()
        };
        let read = limits(TokenAccess::Read);
        assert_eq!(
            read.session_role("w", "p", theirs, push),
            Some(SessionRole::Viewer)
        );
        let run = limits(TokenAccess::Run);
        // Its person's own sessions stay theirs to stop and resume; others'
        // it messages.
        assert_eq!(
            run.session_role("w", "p", theirs, push),
            Some(SessionRole::Owner)
        );
        assert_eq!(
            run.session_role("w", "p", admin(), push),
            Some(SessionRole::Editor)
        );
        assert_eq!(
            limits(TokenAccess::Full).session_role("w", "p", admin(), push),
            Some(SessionRole::Owner)
        );
    }

    #[test]
    fn a_github_cap_leaves_agents_that_may_do_more_read_only() {
        let reviews = Limits {
            access: TokenAccess::Run,
            places: None,
            github_access: Some(GithubAccess::Comment),
        };
        let theirs = SessionAccess {
            is_creator: true,
            ..admin()
        };
        assert_eq!(
            reviews.session_role("w", "p", theirs, GithubAccess::Push),
            Some(SessionRole::Viewer)
        );
        assert_eq!(
            reviews.session_role("w", "p", theirs, GithubAccess::Comment),
            Some(SessionRole::Owner)
        );
        assert_eq!(
            reviews.session_role("w", "p", theirs, GithubAccess::Read),
            Some(SessionRole::Owner)
        );
    }

    #[test]
    fn every_limited_token_reads_and_reaches_nothing_else() {
        for access in TokenAccess::ALL {
            for (method, path) in [
                ("GET", "auth/me"),
                ("POST", "auth/logout"),
                ("GET", "projects"),
                ("GET", "projects/web"),
                ("GET", "projects/web/memories"),
                ("GET", "projects/web/review-agents"),
                ("GET", "projects/web/reviews"),
                ("GET", "sessions"),
                ("GET", "sessions/ses_1/events"),
                ("GET", "sessions/ses_1/ws"),
                ("GET", "sessions/ses_1/attachments/att_1"),
                ("GET", "workspaces"),
            ] {
                let segs: Vec<&str> = path.split('/').collect();
                assert!(
                    allows(access, method, &segs).is_ok(),
                    "{access:?} {method} {path}"
                );
            }
            for (method, path) in [
                ("POST", "auth/tokens"),
                ("GET", "auth/tokens"),
                ("DELETE", "auth/tokens/tok_1"),
                ("PATCH", "auth/me"),
                ("POST", "auth/password"),
                ("GET", "me/export"),
                ("GET", "me/claude"),
                ("POST", "me/delete"),
                ("PATCH", "me/two-factor"),
                ("POST", "projects"),
                ("POST", "workspaces"),
                ("GET", "workspaces/wsp_1"),
                ("GET", "workspaces/wsp_1/members"),
                ("POST", "workspaces/wsp_1/invitations"),
                ("PATCH", "workspaces/wsp_1"),
                ("GET", "admin/users"),
                ("GET", "workspaces/wsp_1/avatar"),
                ("POST", "auth/device/KFTW-QXPL"),
                ("GET", "runners"),
                ("GET", "feed/ws"),
                ("GET", "github/repos"),
                ("POST", "runner/drain"),
            ] {
                let segs: Vec<&str> = path.split('/').collect();
                assert!(
                    allows(access, method, &segs).is_err(),
                    "{access:?} {method} {path}"
                );
            }
        }
    }

    #[test]
    fn each_access_adds_what_it_says() {
        let can = |access, method, path: &str| {
            let segs: Vec<&str> = path.split('/').collect();
            allows(access, method, &segs).is_ok()
        };
        use TokenAccess::{Full, Read, Run};
        for (method, path) in [
            ("POST", "projects/web/sessions"),
            ("POST", "projects/web/attachments"),
            ("POST", "sessions/ses_1/messages"),
            ("POST", "sessions/ses_1/stop"),
            ("PATCH", "sessions/ses_1"),
            ("POST", "projects/web/memories"),
            ("PATCH", "memories/mem_1"),
            ("POST", "projects/web/reviews"),
        ] {
            assert!(!can(Read, method, path), "{method} {path}");
            assert!(can(Run, method, path), "{method} {path}");
            assert!(can(Full, method, path), "{method} {path}");
        }
        for (method, path) in [
            ("PATCH", "projects/web"),
            ("DELETE", "projects/web"),
            ("PUT", "projects/web/secrets/TOKEN"),
            ("GET", "projects/web/secrets"),
            ("POST", "projects/web/members"),
            ("DELETE", "projects/web/repos/acme/lib"),
            ("POST", "sessions/ses_1/shares"),
            ("DELETE", "memories/mem_1"),
            ("DELETE", "retrospectives/ret_1"),
            ("PATCH", "projects/web/review-agents"),
        ] {
            assert!(!can(Run, method, path), "{method} {path}");
            assert!(can(Full, method, path), "{method} {path}");
        }
    }

    #[test]
    fn sessions_get_what_their_project_and_token_allow() {
        use GithubAccess::{Comment, Push, Read};
        assert_eq!(session_github(None, None, None), Ok(Push));
        assert_eq!(session_github(None, Some(Comment), None), Ok(Comment));
        assert_eq!(session_github(None, None, Some(Read)), Ok(Read));
        assert_eq!(session_github(Some(Read), Some(Comment), None), Ok(Read));
        let refused = |asked, project, token| {
            session_github(Some(asked), project, token)
                .err()
                .map(|e| e.message)
                .unwrap_or_default()
        };
        assert!(
            refused(Push, Some(Comment), None)
                .contains("This project's agents may at most comment")
        );
        assert!(
            refused(Comment, None, Some(Read)).contains("access token's sessions may at most read")
        );
        assert_eq!(effective(None, None), Push);
        assert_eq!(effective(Some(Push), Some(Read)), Read);
        assert_eq!(effective(Some(Comment), Some(Push)), Comment);
    }
}
