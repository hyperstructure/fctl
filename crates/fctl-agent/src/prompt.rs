//! The text fctl gives Claude: what it appends to the system prompt, and the
//! AGENTS.md at the root of the sandbox's home, beside the checkout.

use fctl_domain::size;
use fctl_proto::runner::{SandboxSize, SessionSpec};

/// `~/AGENTS.md`: how the agent keeps its session's title and description
/// about the work (Claude reads it through `~/.claude/CLAUDE.md`).
pub const AGENTS_MD: &str = r#"# fctl

This sandbox runs an fctl session. People find sessions in fctl's lists by
their titles, and a new session's title is only the first line of its task.
Keep the title and description about the work with `fctl edit`, which talks
to this session directly (no sign-in needed):

```sh
fctl edit --title "Retry failed uploads in the sync worker" \
  --description "Uploads that time out are retried with backoff, and the worker has tests for it."
```

- Name the session as soon as you understand the task, before the work
  itself.
- The title says what the work is, not what was asked: at most 60
  characters, sentence case, no full stop. "Fix the flaky login test", not
  "Can you look at the login test?".
- The description is one or two plain sentences: what the session is doing
  and why.
- Follow-ups that continue the same work keep the title. When a message
  turns the session to different work, rename it for the new scope, or name
  both when it now covers both ("Fix the login redirect; export audit
  logs"), and update the description.
- If `fctl edit` fails, carry on with the task.
"#;

/// `sandbox`: what the sandbox is limited to, when its driver limits it.
pub fn system_prompt(spec: &SessionSpec, workdir: &str, sandbox: Option<SandboxSize>) -> String {
    let mut s = String::new();
    s.push_str("You are running as an autonomous agent in fctl, a software factory. ");
    s.push_str("Your work happens in a sandbox; people follow along live and may message you.\n\n");
    match &spec.project.repo {
        Some(repo) => s.push_str(&format!(
            "- The repository {} is cloned at {workdir}. You are on the branch `{}`, created from `{}`.\n",
            repo.full_name(),
            spec.branch,
            spec.base_branch
        )),
        None => s.push_str(&format!("- Your working directory is {workdir}. No GitHub repository is linked.\n")),
    }
    if let Some(size) = sandbox {
        // With Docker, nproc and free show the whole VM while cgroups hold
        // the sandbox to its size, so build tools would start too many jobs.
        s.push_str(&format!(
            "- This sandbox has {} and {} of memory ($FCTL_SANDBOX_CPUS, $FCTL_SANDBOX_MEMORY_MB), even where `nproc` and `free` show more. Size parallel builds and test runs to these limits (for example `make -j{}`): when the sandbox runs out of memory, the kernel kills a process, which then exits with code 137.\n",
            size::cpus(size.cpus),
            size::memory(size.memory_mb.into()),
            size.cpus
        ));
    }
    s.push_str("- Work in small, verified steps: read before you change, and run the project's tests and linters.\n");
    s.push_str("- Commit as you go with clear messages. fctl pushes new commits on your branch after every turn; never force-push and never rewrite published history.\n");
    if spec.project.repo.is_some() && spec.project.settings.auto_pull_request {
        s.push_str(&format!(
            "- When the task is complete, open a pull request against `{}` with `gh pr create` (gh is signed in). Describe what changed and why, how you verified it, and link the session: {}\n",
            spec.base_branch, spec.web_url
        ));
    }
    s.push_str("- Messages from people other than the session's starter begin with their name.\n");
    s.push_str("- Never print, log or commit secrets. Project secrets are available as environment variables.\n");
    s.push_str("- If you are blocked on a decision only a person can make, say so plainly and stop; someone will answer.\n");
    if let Some(extra) = spec
        .project
        .settings
        .instructions
        .as_deref()
        .filter(|t| !t.trim().is_empty())
    {
        s.push_str("\nProject instructions:\n");
        s.push_str(extra.trim());
        s.push('\n');
    }
    s
}

/// The first turn when a resumed session lost its conversation: its stopped
/// sandbox was removed (the host's `keep_stopped_hours`), so Claude starts a
/// new one on the same branch, from the session's original task.
pub fn fresh_resume(branch: &str, task: Option<&str>) -> String {
    let mut s = format!(
        "This session is resuming in a new sandbox. The old one was removed after the session had been stopped for a while, so your earlier conversation is gone. The repository is checked out on `{branch}` with everything that was pushed to it."
    );
    if let Some(task) = task.map(str::trim).filter(|t| !t.is_empty()) {
        s.push_str("\n\nThe session's original task was:\n\n");
        s.push_str(task);
    }
    s.push_str("\n\nCatch up on what the branch already contains, then reply with a short summary of where things stand. If a message follows, act on it.");
    s
}

/// A user turn as Claude sees it: messages from others carry their name.
pub fn user_turn(text: &str, author: Option<&str>, starter: &str) -> String {
    match author {
        Some(name) if name != starter => format!("{name} wrote:\n\n{text}"),
        _ => text.to_owned(),
    }
}

/// The sandbox's size for Claude, its tools and the setup script.
pub fn sandbox_env(sandbox: Option<SandboxSize>) -> Vec<(String, String)> {
    sandbox
        .map(|s| {
            vec![
                ("FCTL_SANDBOX_CPUS".to_owned(), s.cpus.to_string()),
                ("FCTL_SANDBOX_MEMORY_MB".to_owned(), s.memory_mb.to_string()),
            ]
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::runner::{
        ClaudeCredential, ClaudeCredentialKind, GitIdentity, ProjectSpec, SessionCredentials,
    };
    use fctl_proto::session::{ProjectSettings, UserRef};
    use fctl_proto::{CredentialId, ProjectId, SessionId, UserId};

    fn spec() -> SessionSpec {
        SessionSpec {
            session_id: SessionId("ses_1".into()),
            number: 1,
            title: "Build it".into(),
            project: ProjectSpec {
                id: ProjectId("prj_1".into()),
                slug: "app".into(),
                name: "App".into(),
                repo: None,
                default_branch: "main".into(),
                settings: ProjectSettings::default(),
            },
            branch: "fctl/1".into(),
            base_branch: "main".into(),
            prompt: Some("Build it".into()),
            task: Some("Build it".into()),
            author: UserRef {
                id: UserId("usr_1".into()),
                name: "Ada".into(),
                avatar_url: None,
            },
            claude_session_id: "uuid".into(),
            resume: false,
            model: None,
            effort: None,
            secrets: Default::default(),
            credentials: SessionCredentials {
                claude: ClaudeCredential {
                    id: CredentialId("crd_1".into()),
                    kind: ClaudeCredentialKind::SetupToken,
                    access_token: "token".into(),
                    scopes: vec![],
                    expires_at: None,
                    subscription_type: None,
                    account: None,
                },
                github: None,
            },
            git_author: GitIdentity {
                name: "fctl".into(),
                email: "agent@fctl.invalid".into(),
            },
            co_authors: vec![],
            web_url: "https://app.fctl.sh/s/ses_1".into(),
            idle_timeout_secs: 7200,
            run_id: None,
        }
    }

    #[test]
    fn tells_claude_the_sandboxs_size() {
        let size = SandboxSize {
            cpus: 4,
            memory_mb: 8192,
        };
        let text = system_prompt(&spec(), "/work", Some(size));
        assert!(text.contains("This sandbox has 4 CPUs and 8 GiB of memory"));
        assert!(text.contains("`make -j4`"));
        assert!(!system_prompt(&spec(), "/work", None).contains("This sandbox has"));
        assert_eq!(
            sandbox_env(Some(size)),
            [
                ("FCTL_SANDBOX_CPUS".to_owned(), "4".to_owned()),
                ("FCTL_SANDBOX_MEMORY_MB".to_owned(), "8192".to_owned())
            ]
        );
        assert!(sandbox_env(None).is_empty());
    }

    #[test]
    fn fresh_resume_names_the_branch_and_task() {
        let bare = fresh_resume("fctl/7", None);
        assert!(bare.contains("`fctl/7`"));
        assert!(!bare.contains("original task"));
        let text = fresh_resume("fctl/7", Some(" Add a dark mode toggle "));
        assert!(text.contains("original task was:\n\nAdd a dark mode toggle\n"));
    }

    #[test]
    fn agents_md_shows_how_to_name_the_session() {
        assert!(AGENTS_MD.contains("fctl edit --title"));
        assert!(AGENTS_MD.contains("--description"));
        assert!(AGENTS_MD.contains("rename it for the new scope"));
    }

    #[test]
    fn attributes_other_people() {
        assert_eq!(user_turn("hi", Some("Ada"), "Ada"), "hi");
        assert_eq!(user_turn("hi", Some("Bob"), "Ada"), "Bob wrote:\n\nhi");
        assert_eq!(user_turn("continue", None, "Ada"), "continue");
    }
}
