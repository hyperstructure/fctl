//! The text fctl appends to Claude's system prompt.

use fctl_proto::runner::SessionSpec;

pub fn system_prompt(spec: &SessionSpec, workdir: &str) -> String {
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
    fn attributes_other_people() {
        assert_eq!(user_turn("hi", Some("Ada"), "Ada"), "hi");
        assert_eq!(user_turn("hi", Some("Bob"), "Ada"), "Bob wrote:\n\nhi");
        assert_eq!(user_turn("continue", None, "Ada"), "continue");
    }
}
