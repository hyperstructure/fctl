//! Releases from Conventional Commits.
//!
//! `cargo xtask release` reads the commits since the last `v*` tag, picks the
//! next version from them, writes CHANGELOG.md and the workspace version,
//! and makes the release commit and tag (the Release workflow then pushes
//! them and publishes). `cargo xtask notes <tag>` renders one release's notes,
//! and `cargo xtask lint-commit` checks a subject (CI runs it on pull request
//! titles, which become the squash-merged commits).
//!
//! Versions: a breaking change bumps the major version (the minor one before
//! 1.0), a `feat` the minor one, anything else the patch.

use crate::util::{self, output, run};
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

/// Commit types, in the order their sections appear in the notes.
pub const TYPES: &[(&str, &str)] = &[
    ("feat", "Features"),
    ("fix", "Fixes"),
    ("perf", "Performance"),
    ("docs", "Documentation"),
    ("refactor", "Other changes"),
    ("test", "Other changes"),
    ("build", "Other changes"),
    ("ci", "Other changes"),
    ("chore", "Other changes"),
    ("revert", "Other changes"),
];

const MAX_SUBJECT: usize = 72;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// The Conventional Commit type, or `None` for other subjects.
    pub kind: Option<String>,
    pub scope: Option<String>,
    pub breaking: bool,
    pub text: String,
    /// The pull request a squash merge names: `… (#14)`.
    pub pr: Option<u32>,
    pub sha: String,
}

/// Splits `type(scope)!: text`; `None` if the subject is not conventional.
fn split(subject: &str) -> Option<(String, Option<String>, bool, String)> {
    let (head, text) = subject.split_once(": ")?;
    let (head, bang) = match head.strip_suffix('!') {
        Some(h) => (h, true),
        None => (head, false),
    };
    let (kind, scope) = match head.split_once('(') {
        Some((k, rest)) => (k, Some(rest.strip_suffix(')')?)),
        None => (head, None),
    };
    let word = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '/')
    };
    if !word(kind) || scope.is_some_and(|s| !word(s)) || text.trim().is_empty() {
        return None;
    }
    Some((
        kind.to_owned(),
        scope.map(str::to_owned),
        bang,
        text.trim().to_owned(),
    ))
}

/// `text (#14)` → (`text`, 14).
fn strip_pr(text: &str) -> (String, Option<u32>) {
    if let Some(open) = text.rfind(" (#")
        && let Some(num) = text[open + 3..].strip_suffix(')')
        && let Ok(n) = num.parse()
    {
        return (text[..open].trim_end().to_owned(), Some(n));
    }
    (text.to_owned(), None)
}

pub fn parse(sha: &str, subject: &str, body: &str) -> Change {
    let breaking_body = body.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("BREAKING CHANGE:") || l.starts_with("BREAKING-CHANGE:")
    });
    let (subject, pr) = strip_pr(subject.trim());
    match split(&subject) {
        Some((kind, scope, bang, text)) if TYPES.iter().any(|(t, _)| *t == kind) => Change {
            kind: Some(kind),
            scope,
            breaking: bang || breaking_body,
            text,
            pr,
            sha: sha.to_owned(),
        },
        _ => Change {
            kind: None,
            scope: None,
            breaking: breaking_body,
            text: subject,
            pr,
            sha: sha.to_owned(),
        },
    }
}

/// Why `subject` is not a good Conventional Commit subject, if it is not.
pub fn lint(subject: &str) -> Result<(), String> {
    let (subject, _) = strip_pr(subject.trim());
    let types = TYPES.iter().map(|(t, _)| *t).collect::<Vec<_>>().join(", ");
    let Some((kind, _, _, text)) = split(&subject) else {
        return Err(format!(
            "`{subject}` is not `type(scope): what changed`; types: {types}"
        ));
    };
    if !TYPES.iter().any(|(t, _)| *t == kind) {
        return Err(format!(
            "`{kind}` is not a commit type; use one of: {types}"
        ));
    }
    if text.ends_with('.') {
        return Err("drop the full stop at the end".into());
    }
    if subject.chars().count() > MAX_SUBJECT {
        return Err(format!(
            "keep it to {MAX_SUBJECT} characters ({} now); details go in the body",
            subject.chars().count()
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let mut p = s.trim().trim_start_matches('v').splitn(3, '.');
        Some(Self(
            p.next()?.parse().ok()?,
            p.next()?.parse().ok()?,
            p.next()?.parse().ok()?,
        ))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bump {
    Patch,
    Minor,
    Major,
}

impl Bump {
    pub fn parse(s: &str) -> Result<Option<Self>> {
        Ok(match s {
            "auto" => None,
            "patch" => Some(Self::Patch),
            "minor" => Some(Self::Minor),
            "major" => Some(Self::Major),
            other => bail!("--bump is auto, patch, minor or major, not {other}"),
        })
    }

    /// What the changes call for.
    pub fn of(changes: &[Change]) -> Self {
        if changes.iter().any(|c| c.breaking) {
            Self::Major
        } else if changes.iter().any(|c| c.kind.as_deref() == Some("feat")) {
            Self::Minor
        } else {
            Self::Patch
        }
    }

    pub fn apply(self, v: Version) -> Version {
        match self {
            // Before 1.0, breaking changes bump the minor version.
            Self::Major if v.0 == 0 => Version(0, v.1 + 1, 0),
            Self::Major => Version(v.0 + 1, 0, 0),
            Self::Minor => Version(v.0, v.1 + 1, 0),
            Self::Patch => Version(v.0, v.1, v.2 + 1),
        }
    }
}

fn is_release_commit(c: &Change) -> bool {
    (c.kind.as_deref() == Some("chore") && c.scope.as_deref() == Some("release"))
        // Before Conventional Commits: "Release 0.1.2".
        || (c.kind.is_none()
            && c.text
                .strip_prefix("Release ")
                .is_some_and(|v| Version::parse(v).is_some()))
}

/// Sentence case for the notes, except for names written in lowercase
/// (`fctl`, `keep_stopped_hours`, `` `code` ``).
fn capitalized(text: &str) -> String {
    let first = text.split_whitespace().next().unwrap_or_default();
    let keep = first.starts_with("fctl")
        || first.starts_with('`')
        || first.contains(['_', '.', '/'])
        || first.chars().skip(1).any(|c| c.is_ascii_uppercase());
    match text.chars().next() {
        Some(c) if !keep => c.to_uppercase().collect::<String>() + &text[c.len_utf8()..],
        _ => text.to_owned(),
    }
}

fn entry(c: &Change, repo: &str) -> String {
    let text = capitalized(&c.text);
    let scope = c
        .scope
        .as_deref()
        .map(|s| format!("**{s}:** "))
        .unwrap_or_default();
    let link = match c.pr {
        Some(n) => format!("[#{n}]({repo}/pull/{n})"),
        None => {
            let short = c.sha.get(..7).unwrap_or(&c.sha);
            format!("[{short}]({repo}/commit/{})", c.sha)
        }
    };
    format!("- {scope}{text} ({link})\n")
}

/// The notes for one release: sections by type, breaking changes first.
pub fn notes(changes: &[Change], repo: &str) -> String {
    let changes: Vec<&Change> = changes.iter().filter(|c| !is_release_commit(c)).collect();
    let mut out = String::new();
    let mut section = |title: &str, items: Vec<&&Change>| {
        if items.is_empty() {
            return;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("### {title}\n\n"));
        for c in items {
            out.push_str(&entry(c, repo));
        }
    };
    section(
        "Breaking changes",
        changes.iter().filter(|c| c.breaking).collect(),
    );
    let mut titles: Vec<&str> = Vec::new();
    for (_, title) in TYPES {
        if !titles.contains(title) {
            titles.push(title);
        }
    }
    for title in titles {
        section(
            title,
            changes
                .iter()
                .filter(|c| !c.breaking)
                .filter(|c| match c.kind.as_deref() {
                    Some(k) => TYPES.iter().any(|(t, s)| *t == k && *s == title),
                    None => title == "Other changes",
                })
                .collect(),
        );
    }
    if out.is_empty() {
        out.push_str("No changes worth noting.\n");
    }
    out
}

/// CHANGELOG.md with `section` added above the newest release.
pub fn changelog_with(existing: &str, section: &str) -> String {
    const INTRO: &str = "# Changelog\n\nWhat changed in each fctl release, from its Conventional Commits \
                         (`cargo xtask release` writes this file).\n";
    let body = match existing.find("\n## ") {
        Some(i) => &existing[i + 1..],
        None => "",
    };
    let head = if existing.starts_with("# Changelog") {
        existing[..existing.find("\n## ").map_or(existing.len(), |i| i + 1)].trim_end()
    } else {
        INTRO.trim_end()
    };
    format!("{head}\n\n{}\n\n{body}", section.trim_end())
        .trim_end()
        .to_owned()
        + "\n"
}

// ───────────────────────────── git ─────────────────────────────

fn git(root: &Path) -> Command {
    let mut c = Command::new("git");
    c.current_dir(root);
    c
}

/// The newest `v*` tag reachable from `rev`, if any.
pub fn last_tag(root: &Path, rev: &str) -> Result<Option<String>> {
    let out = git(root)
        .args([
            "describe",
            "--tags",
            "--abbrev=0",
            "--match",
            "v[0-9]*.[0-9]*.[0-9]*",
            rev,
        ])
        .stderr(std::process::Stdio::null())
        .output()?;
    Ok(out
        .status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()))
}

/// Commits in `range` (oldest first), parsed.
pub fn changes(root: &Path, range: &str) -> Result<Vec<Change>> {
    let log = output(git(root).args([
        "log",
        "--no-merges",
        "--reverse",
        "--format=%H%x1f%s%x1f%b%x1e",
        range,
    ]))?;
    Ok(log
        .split('\x1e')
        .filter_map(|rec| {
            let mut f = rec.trim_start_matches('\n').splitn(3, '\x1f');
            let sha = f.next()?.trim();
            (!sha.is_empty()).then(|| parse(sha, f.next().unwrap_or(""), f.next().unwrap_or("")))
        })
        .collect())
}

/// `https://github.com/owner/repo` for links.
pub fn repo_url(root: &Path) -> String {
    if let (Ok(server), Ok(repo)) = (
        std::env::var("GITHUB_SERVER_URL"),
        std::env::var("GITHUB_REPOSITORY"),
    ) {
        return format!("{server}/{repo}");
    }
    let origin = output(git(root).args(["remote", "get-url", "origin"])).unwrap_or_default();
    let origin = origin.trim().trim_end_matches(".git");
    match origin
        .strip_prefix("git@github.com:")
        .or_else(|| origin.strip_prefix("https://github.com/"))
    {
        Some(path) => format!("https://github.com/{path}"),
        None => "https://github.com/hyperstructure/fctl".into(),
    }
}

fn today() -> Result<String> {
    let out = output(Command::new("date").args(["-u", "+%Y-%m-%d"]))?;
    Ok(out.trim().to_owned())
}

/// `cargo xtask notes [<tag>]`: one release's notes (default: what is
/// unreleased since the last tag).
pub fn print_notes(root: &Path, tag: Option<String>) -> Result<()> {
    let repo = repo_url(root);
    let range = match &tag {
        Some(t) => match last_tag(root, &format!("{t}^"))? {
            Some(prev) => format!("{prev}..{t}"),
            None => t.clone(),
        },
        None => match last_tag(root, "HEAD")? {
            Some(prev) => format!("{prev}..HEAD"),
            None => "HEAD".into(),
        },
    };
    print!("{}", notes(&changes(root, &range)?, &repo));
    Ok(())
}

pub struct ReleaseOpts {
    pub bump: Option<Bump>,
    pub dry_run: bool,
}

/// `cargo xtask release`: the release commit and tag, locally.
pub fn release(root: &Path, opts: ReleaseOpts) -> Result<String> {
    let current = util::version(root)?;
    let current_v = Version::parse(&current).context("the workspace version is not x.y.z")?;
    let prev = last_tag(root, "HEAD")?;
    let range = prev
        .as_ref()
        .map_or("HEAD".into(), |t| format!("{t}..HEAD"));
    let list = changes(root, &range)?;
    if list.iter().all(is_release_commit) {
        bail!(
            "nothing to release since {}",
            prev.as_deref().unwrap_or("the start")
        );
    }
    // The last release's version is the base, whatever Cargo.toml says.
    let base = prev
        .as_deref()
        .and_then(Version::parse)
        .unwrap_or(current_v);
    let bump = opts.bump.unwrap_or_else(|| Bump::of(&list));
    let next = bump.apply(base);
    let tag = format!("v{next}");
    let date = today()?;
    let body = notes(&list, &repo_url(root));
    let section = format!("## {next} ({date})\n\n{body}");
    println!(
        "fctl {next} ({bump:?} bump since {})\n",
        prev.as_deref().unwrap_or("the start")
    );
    print!("{body}");
    if let Ok(summary) = std::env::var("GITHUB_STEP_SUMMARY") {
        let _ = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(summary)
            .and_then(|mut f| {
                use std::io::Write;
                write!(f, "## fctl {next}\n\n{body}\n")
            });
    }
    if opts.dry_run {
        println!("\n(dry run: nothing changed)");
        return Ok(tag);
    }
    if !output(git(root).args(["status", "--porcelain"]))?
        .trim()
        .is_empty()
    {
        bail!("the working tree has changes; commit or stash them first");
    }
    let exists = git(root)
        .args(["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")])
        .stdout(std::process::Stdio::null())
        .status()?
        .success();
    if exists {
        bail!("{tag} exists already");
    }
    set_version(root, &current, &next.to_string())?;
    let changelog = root.join("CHANGELOG.md");
    let existing = std::fs::read_to_string(&changelog).unwrap_or_default();
    std::fs::write(&changelog, changelog_with(&existing, &section))?;
    run(git(root).args(["add", "Cargo.toml", "Cargo.lock", "CHANGELOG.md"]))?;
    run(git(root).args(["commit", "-q", "-m", &format!("chore(release): {tag}")]))?;
    run(git(root).args(["tag", "-a", &tag, "-m", &format!("fctl {next}")]))?;
    println!("\nCommitted and tagged {tag}. Push both: git push origin HEAD {tag}");
    Ok(tag)
}

/// Rewrites `[workspace.package] version` and the lockfile's workspace crates.
fn set_version(root: &Path, from: &str, to: &str) -> Result<()> {
    let path = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&path)?;
    let old = format!("\nversion = \"{from}\"\n");
    if text.matches(&old).count() != 1 {
        bail!("expected one `version = \"{from}\"` line in Cargo.toml");
    }
    std::fs::write(
        &path,
        text.replacen(&old, &format!("\nversion = \"{to}\"\n"), 1),
    )?;
    run(Command::new("cargo")
        .args(["update", "--workspace", "--offline", "--quiet"])
        .current_dir(root))
}

/// `cargo xtask lint-commit <subject>` (or `--range a..b` to check commits).
pub fn lint_commits(root: &Path, subject: Option<String>, range: Option<String>) -> Result<()> {
    let mut bad = Vec::new();
    if let Some(s) = subject
        && let Err(why) = lint(&s)
    {
        bad.push(format!("{s}\n  {why}"));
    }
    if let Some(r) = range {
        let log = output(git(root).args(["log", "--no-merges", "--format=%h %s", &r]))?;
        for line in log.lines() {
            let (sha, s) = line.split_once(' ').unwrap_or((line, ""));
            if let Err(why) = lint(s) {
                bad.push(format!("{sha} {s}\n  {why}"));
            }
        }
    }
    if bad.is_empty() {
        return Ok(());
    }
    bail!(
        "not Conventional Commits (see .claude/skills/commit/SKILL.md):\n{}",
        bad.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_conventional_subjects() {
        let c = parse("abc1234def", "feat(cli): add fctl self update (#15)", "");
        assert_eq!(c.kind.as_deref(), Some("feat"));
        assert_eq!(c.scope.as_deref(), Some("cli"));
        assert_eq!(c.text, "add fctl self update");
        assert_eq!(c.pr, Some(15));
        assert!(!c.breaking);
        assert!(parse("1", "fix!: drop the v0 protocol", "").breaking);
        assert!(
            parse(
                "1",
                "refactor(proto): rename",
                "x\nBREAKING CHANGE: renamed\n"
            )
            .breaking
        );
        let old = parse("1", "Start session branches untracked (#14)", "");
        assert_eq!((old.kind, old.pr), (None, Some(14)));
        assert_eq!(parse("1", "wip: things", "").kind, None);
        assert_eq!(parse("1", "Feat: Capitalized", "").kind, None);
    }

    #[test]
    fn lints_titles() {
        assert!(lint("feat(host): remove stopped sandboxes after keep_stopped_hours").is_ok());
        assert!(lint("fix: start branches untracked (#14)").is_ok());
        assert!(lint("Start session branches untracked").is_err());
        assert!(lint("feature: something").is_err());
        assert!(lint("fix: trailing stop.").is_err());
        assert!(lint(&format!("docs: {}", "x".repeat(80))).is_err());
        assert!(lint("feat(Web): scope case").is_err());
    }

    #[test]
    fn picks_the_next_version() {
        let ch = |s: &str| parse("1", s, "");
        assert_eq!(Bump::of(&[ch("fix: a"), ch("docs: b")]), Bump::Patch);
        assert_eq!(Bump::of(&[ch("fix: a"), ch("feat: b")]), Bump::Minor);
        assert_eq!(Bump::of(&[ch("feat!: b")]), Bump::Major);
        assert_eq!(Bump::Major.apply(Version(0, 1, 2)), Version(0, 2, 0));
        assert_eq!(Bump::Major.apply(Version(1, 4, 2)), Version(2, 0, 0));
        assert_eq!(Bump::Minor.apply(Version(0, 1, 2)), Version(0, 2, 0));
        assert_eq!(Bump::Patch.apply(Version(0, 1, 2)), Version(0, 1, 3));
        assert_eq!(Version::parse("v0.10.1"), Some(Version(0, 10, 1)));
    }

    #[test]
    fn renders_notes_by_section() {
        let repo = "https://github.com/hyperstructure/fctl";
        let list = vec![
            parse(
                "aaaaaaa1",
                "fix(agent): resume in a fresh sandbox (#20)",
                "",
            ),
            parse("bbbbbbb2", "feat(cli): add fctl self update (#15)", ""),
            parse("ccccccc3", "Start session branches untracked (#14)", ""),
            parse("ddddddd4", "feat(proto)!: drop the v0 uplink", ""),
            parse("eeeeeee5", "chore(release): v0.2.0", ""),
            parse("fffffff6", "ci: lint pull request titles", ""),
        ];
        let text = notes(&list, repo);
        assert_eq!(
            text,
            "### Breaking changes\n\n\
             - **proto:** Drop the v0 uplink ([ddddddd](https://github.com/hyperstructure/fctl/commit/ddddddd4))\n\n\
             ### Features\n\n\
             - **cli:** Add fctl self update ([#15](https://github.com/hyperstructure/fctl/pull/15))\n\n\
             ### Fixes\n\n\
             - **agent:** Resume in a fresh sandbox ([#20](https://github.com/hyperstructure/fctl/pull/20))\n\n\
             ### Other changes\n\n\
             - Start session branches untracked ([#14](https://github.com/hyperstructure/fctl/pull/14))\n\
             - Lint pull request titles ([fffffff](https://github.com/hyperstructure/fctl/commit/fffffff6))\n"
        );
        assert_eq!(
            notes(&[parse("1", "chore(release): v1", "")], repo),
            "No changes worth noting.\n"
        );
        assert_eq!(
            notes(&[parse("1", "Release 0.1.2", "")], repo),
            "No changes worth noting.\n"
        );
        assert_eq!(capitalized("fctl self update"), "fctl self update");
        assert_eq!(capitalized("`--json` everywhere"), "`--json` everywhere");
        assert_eq!(
            capitalized("keep_stopped_hours works"),
            "keep_stopped_hours works"
        );
        assert_eq!(capitalized("iOS app"), "iOS app");
        assert_eq!(capitalized("ägent"), "Ägent");
    }

    #[test]
    fn changelog_sections_go_on_top() {
        let first = changelog_with("", "## 0.2.0 (2026-10-18)\n\n### Fixes\n\n- A\n");
        assert!(first.starts_with("# Changelog\n\nWhat changed"));
        assert!(first.ends_with("## 0.2.0 (2026-10-18)\n\n### Fixes\n\n- A\n"));
        let second = changelog_with(&first, "## 0.3.0 (2026-11-01)\n\n### Features\n\n- B\n");
        let a = second.find("## 0.3.0").unwrap_or(usize::MAX);
        let b = second.find("## 0.2.0").unwrap_or(0);
        assert!(a < b, "{second}");
        assert!(second.contains("- B\n\n## 0.2.0"), "{second}");
        assert_eq!(second.matches("# Changelog").count(), 1);
    }
}
