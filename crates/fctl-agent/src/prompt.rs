//! The text fctl gives Claude: what it appends to the system prompt, and the
//! AGENTS.md at the root of the sandbox's home, beside the checkout.

use crate::workspace::Paths;
use fctl_domain::{files, size};
use fctl_proto::event::Attachment;
use fctl_proto::runner::{SandboxSize, SessionRepo, SessionSpec};
use fctl_proto::session::RepoAccess;
use std::path::PathBuf;

/// `~/AGENTS.md`: how the agent keeps its session's title and description
/// about the work (Claude reads it through `~/.claude/CLAUDE.md`), and, in
/// projects that publish screenshots (`screenshot_days`, how long their
/// links work), how it shows screenshots in pull requests.
pub fn agents_md(screenshot_days: Option<u32>) -> String {
    let mut s = AGENTS_MD.to_owned();
    if let Some(days) = screenshot_days {
        let lasts = if days == 1 {
            "24 hours".to_owned()
        } else {
            format!("{days} days")
        };
        s.push_str(&SCREENSHOTS_MD.replace("{lasts}", &lasts));
    }
    s
}

const AGENTS_MD: &str = r#"# fctl

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

const SCREENSHOTS_MD: &str = r#"
## Screenshots in pull requests

Show how a pull request changes what people see with screenshots in its
description. Attach them with `gh pr create --attach FILE` or
`gh pr edit N --attach FILE` where that works. Here gh has the GitHub App's
token, and GitHub takes uploads only from people, so `--attach` fails with
"unsupported authentication type". Then publish them on fctl, which talks
to this session directly:

```sh
fctl publish before.png after.png
```

It prints a link for each and Markdown that shows it,
`![after.png](https://…)`. Put that in the description and say what each
screenshot shows.

- Anyone who has a link can open it, for {lasts}; then it stops working.
  Publish only what the pull request shows, never secrets, tokens or
  people's details. If you publish one by mistake, say so in your reply:
  people who manage the session can delete it.
- PNG, JPEG, GIF and WebP images of at most 5 MB.
- If `fctl publish` fails, describe the screenshots in words instead.
"#;

/// `paths` say where the other repositories go; `sandbox` what the sandbox
/// is limited to, when its driver limits it.
pub fn system_prompt(
    spec: &SessionSpec,
    workdir: &str,
    paths: &Paths,
    sandbox: Option<SandboxSize>,
) -> String {
    let mut s = String::new();
    s.push_str("You are running as an autonomous agent in fctl, a software factory. ");
    s.push_str("Your work happens in a sandbox; people follow along live and may message you.\n\n");
    let repos = spec.project.repos.as_deref().unwrap_or_default();
    match &spec.project.repo {
        Some(repo) => s.push_str(&format!(
            "- The repository {} is cloned at {workdir}. You are on the branch `{}`, created from `{}`.\n",
            repo.full_name(),
            spec.branch,
            spec.base_branch
        )),
        None if repos.is_empty() => s.push_str(&format!("- Your working directory is {workdir}. No GitHub repository is linked.\n")),
        None => s.push_str(&format!("- Your working directory is {workdir}, an empty git repository.\n")),
    }
    let usable = |access: RepoAccess| {
        repos
            .iter()
            .any(|r| r.access == access && r.unavailable.is_none())
    };
    let (writable, readable) = (usable(RepoAccess::Write), usable(RepoAccess::Read));
    if !repos.is_empty() {
        let dir = paths.repos.display();
        let example = repos
            .iter()
            .find(|r| r.unavailable.is_none() && r.name.is_some())
            .map_or_else(
                || format!("{}/<name>", repos[0].owner),
                |r| format!("{}/{}", r.owner, r.name.as_deref().unwrap_or_default()),
            );
        let also = if spec.project.repo.is_some() {
            "also "
        } else {
            ""
        };
        s.push_str(&format!(
            "- You may {also}use the repositories below. Clone one when you need it, into {dir}/<owner>/<name> (for example `git clone https://github.com/{example} {dir}/{example}`); git is signed in for them:\n"
        ));
        for r in repos {
            s.push_str(&format!("  - {}\n", describe(r, repos)));
        }
        if writable {
            s.push_str(&format!(
                "- In each of those repositories you change, work on a branch named `{}` too: check it out if it exists on the remote, otherwise create it from the default branch before your first commit. fctl pushes it after every turn. Read-only repositories can't be pushed to.\n",
                spec.branch
            ));
        }
        if readable && (writable || spec.project.repo.is_some()) {
            s.push_str(&format!(
                "- gh is signed in for the repositories you can write to. For a read-only one, run it as `GH_TOKEN=\"$(cat {})\" gh …`.\n",
                paths.read_token_file.display()
            ));
        }
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
    if spec.project.settings.auto_pull_request {
        let describe = format!(
            "Describe what changed and why, how you verified it, and link the session: {}",
            spec.web_url
        );
        match (spec.project.repo.is_some(), writable) {
            (true, false) => s.push_str(&format!(
                "- When the task is complete, open a pull request against `{}` with `gh pr create` (gh is signed in). {describe}\n",
                spec.base_branch
            )),
            (true, true) => s.push_str(&format!(
                "- When the task is complete, open a pull request against `{}` with `gh pr create` (gh is signed in), and one in each other repository you changed, against its default branch; link them to each other. {describe}\n",
                spec.base_branch
            )),
            (false, true) => s.push_str(&format!(
                "- When the task is complete, open a pull request in each repository you changed, against its default branch, with `gh pr create` (gh is signed in); link them to each other. {describe}\n"
            )),
            (false, false) => {}
        }
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

/// One of the other repositories, as the system prompt lists it.
fn describe(r: &SessionRepo, all: &[SessionRepo]) -> String {
    let access = match r.access {
        RepoAccess::Write => "read and write",
        RepoAccess::Read => "read only",
    };
    match (&r.name, &r.unavailable) {
        (Some(name), Some(_)) => format!(
            "{}/{name}: not available in this session (the GitHub App can't reach it)",
            r.owner
        ),
        (Some(name), None) => format!("{}/{name} ({access})", r.owner),
        (None, _) if all.iter().any(|o| o.name.is_some()) => {
            format!("any other repository of {} (read only)", r.owner)
        }
        (None, _) => format!("any repository of {} (read only)", r.owner),
    }
}

/// The repositories for the transcript: `acme/lib (read and write), …`.
pub fn repos_line(repos: &[SessionRepo], dir: &str) -> String {
    let list: Vec<String> = repos
        .iter()
        .map(|r| {
            let label = r.grant().label();
            match (r.access, &r.unavailable) {
                (_, Some(_)) => format!("{label} (unavailable)"),
                (RepoAccess::Write, None) => format!("{label} (read and write)"),
                (RepoAccess::Read, None) => format!("{label} (read only)"),
            }
        })
        .collect();
    format!(
        "Other repositories, cloned when the agent needs them into {dir}: {}",
        list.join(", ")
    )
}

/// The first turn when a resumed session lost its conversation: its stopped
/// sandbox was removed (the host's `keep_stopped_hours`), so Claude starts a
/// new one on the same branch, from the session's original task.
pub fn fresh_resume(branch: &str, task: Option<&str>, other_repos: bool) -> String {
    let mut s = format!(
        "This session is resuming in a new sandbox. The old one was removed after the session had been stopped for a while, so your earlier conversation is gone. The repository is checked out on `{branch}` with everything that was pushed to it."
    );
    if other_repos {
        s.push_str(&format!(
            " The other repositories are not cloned yet: clone the ones you worked in again and check out their `{branch}` branches."
        ));
    }
    if let Some(task) = task.map(str::trim).filter(|t| !t.is_empty()) {
        s.push_str("\n\nThe session's original task was:\n\n");
        s.push_str(task);
    }
    s.push_str("\n\nCatch up on what the branch already contains, then reply with a short summary of where things stand. If a message follows, act on it.");
    s
}

/// One attached file, as a message describes it to Claude.
pub struct FileNote<'a> {
    pub attachment: &'a Attachment,
    /// Where it was saved; `None` if it never reached the sandbox.
    pub path: Option<PathBuf>,
    /// Whether the image is also in the message.
    pub inline: bool,
}

/// The list of files that follows a message's text (empty without files).
pub fn files_note(files: &[FileNote<'_>]) -> String {
    if files.is_empty() {
        return String::new();
    }
    let mut s = String::from("Attached files (saved in the sandbox):");
    for f in files {
        let a = f.attachment;
        let mut facts = vec![a.media_type.clone()];
        if let (Some(w), Some(h)) = (a.width, a.height) {
            facts.push(format!("{w}×{h}"));
        }
        facts.push(files::human_size(a.size));
        let facts = facts.join(", ");
        match &f.path {
            Some(p) if f.inline => {
                s.push_str(&format!("\n- {} ({facts}; shown below)", p.display()))
            }
            Some(p) => s.push_str(&format!("\n- {} ({facts})", p.display())),
            None => s.push_str(&format!(
                "\n- {} ({facts}): it did not reach the sandbox",
                a.name
            )),
        }
    }
    s
}

/// A message's text followed by the list of its files.
pub fn with_files(text: &str, note: &str) -> String {
    match (text.trim().is_empty(), note.is_empty()) {
        (_, true) => text.to_owned(),
        (true, false) => note.to_owned(),
        (false, false) => format!("{}\n\n{note}", text.trim_end()),
    }
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
                repos: None,
            },
            branch: "fctl/1".into(),
            base_branch: "main".into(),
            prompt: Some("Build it".into()),
            task: Some("Build it".into()),
            task_attachments: vec![],
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
                github_read: None,
            },
            git_author: GitIdentity {
                name: "fctl".into(),
                email: "agent@fctl.invalid".into(),
            },
            co_authors: vec![],
            co_authored_by: vec![],
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
        let text = system_prompt(&spec(), "/work", &paths(), Some(size));
        assert!(text.contains("This sandbox has 4 CPUs and 8 GiB of memory"));
        assert!(text.contains("`make -j4`"));
        assert!(!system_prompt(&spec(), "/work", &paths(), None).contains("This sandbox has"));
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
        let bare = fresh_resume("fctl/7", None, false);
        assert!(bare.contains("`fctl/7`"));
        assert!(!bare.contains("original task"));
        assert!(!bare.contains("other repositories"));
        let text = fresh_resume("fctl/7", Some(" Add a dark mode toggle "), true);
        assert!(text.contains("original task was:\n\nAdd a dark mode toggle\n"));
        assert!(text.contains("The other repositories are not cloned yet"));
    }

    /// acme/app, with `repos` as the project's other repositories.
    fn spec_with(repos: serde_json::Value) -> SessionSpec {
        let mut v = serde_json::json!({
            "session_id": "ses_test",
            "number": 12,
            "title": "Test",
            "project": {
                "id": "prj_test",
                "slug": "app",
                "name": "App",
                "repo": { "owner": "acme", "name": "app" },
                "default_branch": "main",
                "settings": {}
            },
            "branch": "fctl/12-test",
            "base_branch": "main",
            "author": { "id": "usr_test", "name": "Ada" },
            "claude_session_id": "00000000-0000-0000-0000-000000000000",
            "resume": false,
            "secrets": {},
            "credentials": {
                "claude": { "id": "crd_test", "kind": "setup_token", "access_token": "t", "scopes": [] }
            },
            "git_author": { "name": "fctl", "email": "fctl@example.com" },
            "co_authors": [],
            "web_url": "https://fctl.test/s/ses_test",
            "idle_timeout_secs": 600
        });
        if !repos.is_null() {
            v["project"]["repos"] = repos;
        }
        serde_json::from_value(v).unwrap_or_else(|e| panic!("spec: {e}"))
    }

    fn paths() -> Paths {
        Paths::new(
            std::path::Path::new("/home/agent/workspace"),
            std::path::Path::new("/home/agent"),
            std::path::Path::new("/home/agent/.fctl/state"),
        )
    }

    #[test]
    fn one_repository_reads_as_before() {
        let text = system_prompt(
            &spec_with(serde_json::Value::Null),
            "/home/agent/workspace",
            &paths(),
            None,
        );
        assert!(text.contains("The repository acme/app is cloned at /home/agent/workspace"));
        assert!(text.contains(
            "open a pull request against `main` with `gh pr create` (gh is signed in). Describe"
        ));
        assert!(!text.contains("/home/agent/repos"));
        assert!(!text.contains("GH_TOKEN"));
    }

    #[test]
    fn lists_the_other_repositories_and_where_they_go() {
        let repos = serde_json::json!([
            { "owner": "acme", "name": "lib", "access": "write" },
            { "owner": "acme", "name": "docs", "access": "read" },
            { "owner": "acme", "access": "read" },
            { "owner": "acme", "name": "gone", "access": "write", "unavailable": "The fctl GitHub App can't reach acme/gone." }
        ]);
        let text = system_prompt(&spec_with(repos), "/home/agent/workspace", &paths(), None);
        assert!(text.contains("into /home/agent/repos/<owner>/<name> (for example `git clone https://github.com/acme/lib /home/agent/repos/acme/lib`)"));
        assert!(text.contains("  - acme/lib (read and write)\n"));
        assert!(text.contains("  - acme/docs (read only)\n"));
        assert!(text.contains("  - any other repository of acme (read only)\n"));
        assert!(text.contains("  - acme/gone: not available in this session"));
        assert!(text.contains("work on a branch named `fctl/12-test` too"));
        assert!(text.contains("GH_TOKEN=\"$(cat /home/agent/.fctl/state/github-token-read)\" gh"));
        assert!(text.contains("and one in each other repository you changed, against its default branch; link them to each other."));
    }

    #[test]
    fn read_only_repositories_need_no_branch() {
        let repos = serde_json::json!([{ "owner": "acme", "access": "read" }]);
        let text = system_prompt(&spec_with(repos), "/home/agent/workspace", &paths(), None);
        assert!(text.contains("  - any repository of acme (read only)\n"));
        assert!(
            text.contains("git clone https://github.com/acme/<name> /home/agent/repos/acme/<name>")
        );
        assert!(!text.contains("work on a branch named"));
        assert!(text.contains(
            "open a pull request against `main` with `gh pr create` (gh is signed in). Describe"
        ));
    }

    #[test]
    fn without_a_primary_repository_the_others_are_the_work() {
        let mut spec =
            spec_with(serde_json::json!([{ "owner": "acme", "name": "lib", "access": "write" }]));
        spec.project.repo = None;
        let text = system_prompt(&spec, "/home/agent/workspace", &paths(), None);
        assert!(text.contains(
            "- Your working directory is /home/agent/workspace, an empty git repository.\n"
        ));
        assert!(text.contains("- You may use the repositories below."));
        assert!(text.contains(
            "open a pull request in each repository you changed, against its default branch"
        ));
    }

    #[test]
    fn repositories_for_the_transcript() {
        let repos: Vec<SessionRepo> = serde_json::from_value(serde_json::json!([
            { "owner": "acme", "name": "lib", "access": "write" },
            { "owner": "acme", "access": "read" },
            { "owner": "acme", "name": "gone", "access": "read", "unavailable": "…" }
        ]))
        .unwrap_or_default();
        assert_eq!(
            repos_line(&repos, "/root/repos"),
            "Other repositories, cloned when the agent needs them into /root/repos: acme/lib (read and write), acme/* (read only), acme/gone (unavailable)"
        );
    }

    #[test]
    fn lists_files_after_the_text() {
        let shot = Attachment {
            id: fctl_proto::AttachmentId("att_1".into()),
            name: "shot.png".into(),
            media_type: "image/png".into(),
            size: 245_300,
            width: Some(1280),
            height: Some(720),
        };
        let log = Attachment {
            id: fctl_proto::AttachmentId("att_2".into()),
            name: "build.log".into(),
            media_type: "text/plain".into(),
            size: 1_234_567,
            width: None,
            height: None,
        };
        let note = files_note(&[
            FileNote {
                attachment: &shot,
                path: Some(PathBuf::from("/home/agent/attachments/att_1/shot.png")),
                inline: true,
            },
            FileNote {
                attachment: &log,
                path: None,
                inline: false,
            },
        ]);
        assert_eq!(
            note,
            "Attached files (saved in the sandbox):\n\
             - /home/agent/attachments/att_1/shot.png (image/png, 1280×720, 245 KB; shown below)\n\
             - build.log (text/plain, 1.2 MB): it did not reach the sandbox"
        );
        assert_eq!(
            with_files("Fix this ", &note),
            format!("Fix this\n\n{note}")
        );
        assert_eq!(with_files("  ", &note), note);
        assert_eq!(with_files("Fix this", ""), "Fix this");
        assert_eq!(files_note(&[]), "");
    }

    #[test]
    fn agents_md_shows_how_to_name_the_session() {
        let md = agents_md(None);
        assert!(md.contains("fctl edit --title"));
        assert!(md.contains("--description"));
        assert!(md.contains("rename it for the new scope"));
        assert!(!md.contains("fctl publish"));
    }

    #[test]
    fn agents_md_shows_how_to_publish_screenshots_where_projects_do() {
        let md = agents_md(Some(30));
        assert!(md.starts_with(&agents_md(None)));
        assert!(md.contains("gh pr edit N --attach FILE"));
        assert!(md.contains("unsupported authentication type"));
        assert!(md.contains("fctl publish before.png after.png"));
        assert!(md.contains("for 30 days; then it stops working"));
        assert!(md.contains("If you publish one by mistake, say so"));
        assert!(agents_md(Some(1)).contains("for 24 hours; then"));
    }

    #[test]
    fn attributes_other_people() {
        assert_eq!(user_turn("hi", Some("Ada"), "Ada"), "hi");
        assert_eq!(user_turn("hi", Some("Bob"), "Ada"), "Bob wrote:\n\nhi");
        assert_eq!(user_turn("continue", None, "Ada"), "continue");
    }
}
