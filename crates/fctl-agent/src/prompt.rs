//! The text fctl appends to Claude's system prompt.

use crate::workspace::Paths;
use fctl_proto::runner::{SessionRepo, SessionSpec};
use fctl_proto::session::RepoAccess;

pub fn system_prompt(spec: &SessionSpec, workdir: &str, paths: &Paths) -> String {
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
    let repos = spec.project.repos.as_deref().unwrap_or_default();
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
        s.push_str(&format!(
            "- You may also use the repositories below. Clone one when you need it, into {dir}/<owner>/<name> (for example `git clone https://github.com/{example} {dir}/{example}`); git is signed in for them:\n"
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

/// A user turn as Claude sees it: messages from others carry their name.
pub fn user_turn(text: &str, author: Option<&str>, starter: &str) -> String {
    match author {
        Some(name) if name != starter => format!("{name} wrote:\n\n{text}"),
        _ => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn spec(repos: serde_json::Value) -> SessionSpec {
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
            &spec(serde_json::Value::Null),
            "/home/agent/workspace",
            &paths(),
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
        let text = system_prompt(&spec(repos), "/home/agent/workspace", &paths());
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
        let text = system_prompt(&spec(repos), "/home/agent/workspace", &paths());
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
    fn attributes_other_people() {
        assert_eq!(user_turn("hi", Some("Ada"), "Ada"), "hi");
        assert_eq!(user_turn("hi", Some("Bob"), "Ada"), "Bob wrote:\n\nhi");
        assert_eq!(user_turn("continue", None, "Ada"), "continue");
    }
}
