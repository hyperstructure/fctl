//! The text fctl appends to Claude's system prompt.

use fctl_domain::files;
use fctl_proto::event::Attachment;
use fctl_proto::runner::SessionSpec;
use std::path::PathBuf;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_resume_names_the_branch_and_task() {
        let bare = fresh_resume("fctl/7", None);
        assert!(bare.contains("`fctl/7`"));
        assert!(!bare.contains("original task"));
        let text = fresh_resume("fctl/7", Some(" Add a dark mode toggle "));
        assert!(text.contains("original task was:\n\nAdd a dark mode toggle\n"));
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
    fn attributes_other_people() {
        assert_eq!(user_turn("hi", Some("Ada"), "Ada"), "hi");
        assert_eq!(user_turn("hi", Some("Bob"), "Ada"), "Bob wrote:\n\nhi");
        assert_eq!(user_turn("continue", None, "Ada"), "continue");
    }
}
