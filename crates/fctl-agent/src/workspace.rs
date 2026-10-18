//! The session's repositories: cloning the primary one, branches, the setup
//! script, git state after each turn, pushing, and the credentials git and
//! `gh` use.
//!
//! The agent clones the project's other repositories itself, when it needs
//! them, into `~/repos/<owner>/<name>`. After each turn fctl-agent finds
//! those clones, keeps the work in writable ones on the session branch,
//! pushes it, and looks up its pull request.

use anyhow::{Context, Result, bail};
use fctl_domain::repos;
use fctl_proto::event::{CommitInfo, FileChange, GitState, LogLevel, RepoGitState};
use fctl_proto::runner::{GitIdentity, GithubCredential, SessionSpec};
use fctl_proto::session::{PullRequestRef, RepoAccess, RepoGrant, RepoRef};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

/// Most other repositories reported after a turn, and files per repository.
const MAX_CLONES: usize = 30;
const MAX_CLONE_FILES: usize = 200;

/// Paths fctl-agent manages inside the sandbox.
#[derive(Clone)]
pub struct Paths {
    pub workdir: PathBuf,
    pub home: PathBuf,
    /// Where the agent clones the project's other repositories (`~/repos`).
    pub repos: PathBuf,
    /// The GitHub token that can write: the primary repository and the
    /// other writable ones.
    pub token_file: PathBuf,
    /// The read-only GitHub token.
    pub read_token_file: PathBuf,
    /// The repositories `token_file` covers: `owner/name`, lowercase, one per line.
    pub write_repos_file: PathBuf,
    pub helper: PathBuf,
}

impl Paths {
    pub fn new(workdir: &Path, home: &Path, state: &Path) -> Self {
        Self {
            workdir: workdir.to_path_buf(),
            home: home.to_path_buf(),
            repos: home.join("repos"),
            token_file: state.join("github-token"),
            read_token_file: state.join("github-token-read"),
            write_repos_file: state.join("github-write-repos"),
            helper: state.join("git-credential-fctl"),
        }
    }
}

pub struct Output {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a command to completion, capturing output.
pub async fn run(dir: &Path, home: &Path, program: &str, args: &[&str]) -> Result<Output> {
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .await
        .with_context(|| format!("running {program}"))?;
    Ok(Output {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

async fn git(p: &Paths, args: &[&str]) -> Result<Output> {
    run(&p.workdir, &p.home, "git", args).await
}

async fn git_ok(p: &Paths, args: &[&str]) -> Result<String> {
    git_ok_in(&p.workdir, &p.home, args).await
}

async fn git_ok_in(dir: &Path, home: &Path, args: &[&str]) -> Result<String> {
    let o = run(dir, home, "git", args).await?;
    if !o.ok {
        bail!("git {} failed: {}", args.join(" "), o.stderr.trim());
    }
    Ok(o.stdout)
}

/// Writes the GitHub tokens where git's credential helper and `gh` read
/// them. `gh` takes one token per host: the one that can write, when there
/// is one.
pub fn write_github_tokens(
    p: &Paths,
    write: Option<&GithubCredential>,
    read: Option<&GithubCredential>,
) -> Result<()> {
    if let Some(dir) = p.token_file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if let Some(cred) = write {
        write_private(&p.token_file, &cred.token)?;
    }
    if let Some(cred) = read {
        write_private(&p.read_token_file, &cred.token)?;
    }
    let for_gh = write.or(read.filter(|_| !p.token_file.exists()));
    if let Some(cred) = for_gh {
        let gh_dir = p.home.join(".config/gh");
        std::fs::create_dir_all(&gh_dir)?;
        let hosts = format!(
            "github.com:\n    oauth_token: {}\n    git_protocol: https\n    user: x-access-token\n",
            cred.token
        );
        write_private(&gh_dir.join("hosts.yml"), &hosts)?;
    }
    Ok(())
}

/// The repositories the token that can write covers, for the credential
/// helper: the primary one and the other writable ones.
pub fn write_routes(p: &Paths, spec: &SessionSpec) -> Result<()> {
    let primary = spec.project.repo.iter().map(RepoRef::full_name);
    let writable = spec
        .project
        .repos
        .iter()
        .flatten()
        .filter(|r| r.access == RepoAccess::Write && r.unavailable.is_none())
        .filter_map(|r| r.grant().repo().map(|r| r.full_name()));
    let lines: String = primary
        .chain(writable)
        .map(|r| format!("{}\n", r.to_lowercase()))
        .collect();
    if let Some(dir) = p.write_repos_file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&p.write_repos_file, lines)?;
    Ok(())
}

/// Quotes a path for a shell script.
fn sh_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

/// The credential helper. git sends the repository's path
/// (`credential.useHttpPath`): the repositories in `write_repos_file` get
/// the token that can write, others the read-only one; without a path, or a
/// read-only token, the one that can write.
fn helper_script(p: &Paths) -> String {
    let write = sh_quote(&p.token_file);
    let read = sh_quote(&p.read_token_file);
    let repos = sh_quote(&p.write_repos_file);
    format!(
        r#"#!/bin/sh
# Written by fctl-agent: hands git the session's GitHub token for each repository.
[ "$1" = get ] || exit 0
path=
while IFS= read -r line; do
  case $line in path=*) path=${{line#path=}} ;; esac
done
token=
if [ -n "$path" ] && [ -s {read} ]; then
  owner=${{path%%/*}}
  name=${{path#*/}}
  name=${{name%%/*}}
  repo=$(printf '%s/%s' "$owner" "${{name%.git}}" | tr '[:upper:]' '[:lower:]')
  grep -qxF "$repo" {repos} 2>/dev/null || token=$(cat {read})
fi
[ -n "$token" ] || token=$(cat {write} 2>/dev/null || cat {read} 2>/dev/null)
[ -n "$token" ] || exit 0
echo username=x-access-token
echo "password=$token"
"#
    )
}

fn write_private(path: &Path, contents: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Global git configuration: identity, credential helper, safe defaults.
pub async fn configure_git(p: &Paths, author: &GitIdentity) -> Result<()> {
    if let Some(dir) = p.helper.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&p.helper, helper_script(p))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p.helper, std::fs::Permissions::from_mode(0o700))?;
    }
    let helper_path = p.helper.display().to_string();
    for (k, v) in [
        ("user.name", author.name.as_str()),
        ("user.email", author.email.as_str()),
        ("credential.https://github.com.useHttpPath", "true"),
        ("init.defaultBranch", "main"),
        ("push.autoSetupRemote", "true"),
        ("advice.detachedHead", "false"),
        ("core.quotePath", "false"),
    ] {
        run(&p.home, &p.home, "git", &["config", "--global", k, v]).await?;
    }
    // An empty helper resets the list: only ours answers, once.
    run(
        &p.home,
        &p.home,
        "git",
        &[
            "config",
            "--global",
            "--replace-all",
            "credential.https://github.com.helper",
            "",
        ],
    )
    .await?;
    run(
        &p.home,
        &p.home,
        "git",
        &[
            "config",
            "--global",
            "--add",
            "credential.https://github.com.helper",
            &helper_path,
        ],
    )
    .await?;
    Ok(())
}

pub type LogSink = mpsc::UnboundedSender<(LogLevel, String)>;

/// Clones (or reuses) the primary repository and checks out the session
/// branch. The others are cloned by the agent, into `~/repos`.
pub async fn prepare(p: &Paths, spec: &SessionSpec, log: &LogSink) -> Result<()> {
    std::fs::create_dir_all(&p.workdir)?;
    if spec.project.repos.as_ref().is_some_and(|r| !r.is_empty()) {
        std::fs::create_dir_all(&p.repos)?;
    }
    let Some(repo) = &spec.project.repo else {
        // No repository: an empty git workspace so diffs still work.
        if !p.workdir.join(".git").exists() {
            git_ok(p, &["init", "-q", "-b", &spec.branch]).await?;
            let _ = log.send((
                LogLevel::Info,
                "No repository linked; started an empty workspace".into(),
            ));
        }
        return Ok(());
    };
    if p.workdir.join(".git").exists() {
        let _ = log.send((
            LogLevel::Info,
            format!("Reusing the checkout of {}", repo.full_name()),
        ));
        let f = git(p, &["fetch", "--prune", "origin"]).await?;
        if !f.ok {
            let _ = log.send((
                LogLevel::Warn,
                format!("git fetch failed: {}", f.stderr.trim()),
            ));
        }
        return Ok(());
    }
    let _ = log.send((LogLevel::Info, format!("Cloning {}", repo.full_name())));
    let parent = p.workdir.parent().unwrap_or(Path::new("/"));
    let name = p
        .workdir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace");
    // Clone into an empty directory (the workdir may already exist).
    let is_empty = std::fs::read_dir(&p.workdir)
        .map(|mut d| d.next().is_none())
        .unwrap_or(true);
    let clone = if is_empty {
        std::fs::remove_dir(&p.workdir).ok();
        run(
            parent,
            &p.home,
            "git",
            &["clone", "--quiet", &repo.clone_url(), name],
        )
        .await?
    } else {
        run(
            &p.workdir,
            &p.home,
            "git",
            &["clone", "--quiet", &repo.clone_url(), "."],
        )
        .await?
    };
    if !clone.ok {
        bail!(
            "cloning {} failed: {}",
            repo.full_name(),
            clone.stderr.trim()
        );
    }
    let remote_branch = git(
        p,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/remotes/origin/{}", spec.branch),
        ],
    )
    .await?;
    if remote_branch.ok {
        let _ = log.send((LogLevel::Info, format!("Continuing on {}", spec.branch)));
        git_ok(
            p,
            &[
                "checkout",
                "-q",
                "-B",
                &spec.branch,
                &format!("origin/{}", spec.branch),
            ],
        )
        .await?;
        return Ok(());
    }
    let base = git(
        p,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/remotes/origin/{}", spec.base_branch),
        ],
    )
    .await?;
    if base.ok {
        // Without an upstream, a plain `git push` publishes the branch under
        // its own name (`push.autoSetupRemote`) instead of failing on a
        // mismatch with the base branch.
        git_ok(
            p,
            &[
                "checkout",
                "-q",
                "--no-track",
                "-b",
                &spec.branch,
                &format!("origin/{}", spec.base_branch),
            ],
        )
        .await?;
        let _ = log.send((
            LogLevel::Info,
            format!("Created {} from {}", spec.branch, spec.base_branch),
        ));
    } else {
        // An empty repository: start the branch from nothing.
        git_ok(p, &["checkout", "-q", "--orphan", &spec.branch]).await?;
        let _ = log.send((
            LogLevel::Warn,
            format!(
                "{} has no {} branch; starting {} empty",
                repo.full_name(),
                spec.base_branch,
                spec.branch
            ),
        ));
    }
    Ok(())
}

/// Runs the project's setup script, streaming its output.
pub async fn setup_script(
    p: &Paths,
    script: &str,
    env: &[(String, String)],
    log: &LogSink,
) -> Result<bool> {
    let _ = log.send((LogLevel::Info, "Running the setup script".into()));
    let mut cmd = Command::new("bash");
    cmd.args(["-lc", script])
        .current_dir(&p.workdir)
        .env("HOME", &p.home)
        .env("CI", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().context("starting bash for the setup script")?;
    let stdout = child.stdout.take().context("stdout")?;
    let stderr = child.stderr.take().context("stderr")?;
    let l1 = log.clone();
    let l2 = log.clone();
    let a = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = l1.send((LogLevel::Info, line));
        }
    });
    let b = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = l2.send((LogLevel::Warn, line));
        }
    });
    let status = child.wait().await?;
    let _ = a.await;
    let _ = b.await;
    Ok(status.success())
}

/// The primary repository's state relative to the base branch.
pub async fn state(p: &Paths, spec: &SessionSpec) -> Result<GitState> {
    state_at(&p.workdir, &p.home, &spec.base_branch).await
}

/// A clone's state relative to `base`, a branch of its `origin`.
pub async fn state_at(dir: &Path, home: &Path, base: &str) -> Result<GitState> {
    if !dir.join(".git").exists() {
        return Ok(GitState::default());
    }
    let branch = git_ok_in(dir, home, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .ok()
        .map(|s| s.trim().to_owned());
    let head = git_ok_in(dir, home, &["rev-parse", "HEAD"])
        .await
        .ok()
        .map(|s| s.trim().to_owned());
    let base_ref = format!("origin/{base}");
    let has_base = run(
        dir,
        home,
        "git",
        &["rev-parse", "--verify", "--quiet", &base_ref],
    )
    .await?
    .ok;
    let (behind, ahead) = if has_base && head.is_some() {
        let counts = git_ok_in(
            dir,
            home,
            &[
                "rev-list",
                "--left-right",
                "--count",
                &format!("{base_ref}...HEAD"),
            ],
        )
        .await
        .unwrap_or_default();
        let mut it = counts
            .split_whitespace()
            .filter_map(|n| n.parse::<u32>().ok());
        (it.next().unwrap_or(0), it.next().unwrap_or(0))
    } else {
        (
            0,
            git_ok_in(dir, home, &["rev-list", "--count", "HEAD"])
                .await
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0),
        )
    };
    let mut files: Vec<FileChange> = Vec::new();
    // Without a base branch, everything is new: diff against the empty tree.
    const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
    let diff_target = if has_base {
        base_ref.clone()
    } else {
        EMPTY_TREE.to_owned()
    };
    if head.is_some() {
        let numstat = run(dir, home, "git", &["diff", "--numstat", "-M", &diff_target]).await?;
        let names = run(
            dir,
            home,
            "git",
            &["diff", "--name-status", "-M", &diff_target],
        )
        .await?;
        let statuses: Vec<(String, String)> = names
            .stdout
            .lines()
            .filter_map(|l| {
                let mut parts = l.split('\t');
                let code = parts.next()?.chars().next()?;
                let path = parts.next_back()?.to_owned();
                let status = match code {
                    'A' => "added",
                    'D' => "deleted",
                    'R' => "renamed",
                    _ => "modified",
                };
                Some((path, status.to_owned()))
            })
            .collect();
        for line in numstat.stdout.lines() {
            let mut parts = line.split('\t');
            let (Some(a), Some(d), Some(path)) = (parts.next(), parts.next(), parts.next_back())
            else {
                continue;
            };
            let path = path.to_owned();
            let status = statuses
                .iter()
                .find(|(p2, _)| *p2 == path)
                .map_or("modified".to_owned(), |(_, s)| s.clone());
            files.push(FileChange {
                path,
                status,
                additions: a.parse().ok(),
                deletions: d.parse().ok(),
            });
        }
    }
    let porcelain = run(dir, home, "git", &["status", "--porcelain"]).await?;
    let mut uncommitted = 0u32;
    for line in porcelain.stdout.lines() {
        uncommitted += 1;
        if let Some(path) = line.strip_prefix("?? ") {
            let path = path.trim_matches('"').to_owned();
            if !path.ends_with('/') && !files.iter().any(|f| f.path == path) {
                let lines = std::fs::read_to_string(dir.join(&path))
                    .map(|t| t.lines().count() as u32)
                    .ok();
                files.push(FileChange {
                    path,
                    status: "untracked".into(),
                    additions: lines,
                    deletions: Some(0),
                });
            }
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.truncate(500);
    let last_commit = git_ok_in(dir, home, &["log", "-1", "--format=%H%x00%s"])
        .await
        .ok()
        .and_then(|s| {
            let (sha, subject) = s.trim().split_once('\0')?;
            Some(CommitInfo {
                sha: sha.to_owned(),
                subject: subject.to_owned(),
            })
        });
    let remote_head = match &branch {
        Some(b) => git_ok_in(
            dir,
            home,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/remotes/origin/{b}"),
            ],
        )
        .await
        .ok()
        .map(|s| s.trim().to_owned()),
        None => None,
    };
    Ok(GitState {
        branch: branch.clone(),
        base: Some(base.to_owned()),
        pushed: head.is_some() && remote_head == head,
        head,
        ahead,
        behind,
        files,
        uncommitted,
        last_commit,
        pull_request: None,
        repos: None,
    })
}

/// Pushes the checked-out branch when it has commits the remote lacks.
pub async fn push(dir: &Path, home: &Path, st: &GitState) -> Result<bool> {
    let Some(branch) = st.branch.as_deref() else {
        return Ok(false);
    };
    if st.pushed || st.head.is_none() || branch == "HEAD" {
        return Ok(false);
    }
    let o = run(
        dir,
        home,
        "git",
        &["push", "--quiet", "-u", "origin", branch],
    )
    .await?;
    if !o.ok {
        bail!("git push failed: {}", o.stderr.trim());
    }
    Ok(true)
}

/// The pull request of a branch, through `gh` (which finds the repository
/// from the clone's remote).
pub async fn pull_request(dir: &Path, home: &Path, branch: &str) -> Option<PullRequestRef> {
    let o = run(
        dir,
        home,
        "gh",
        &[
            "pr",
            "view",
            branch,
            "--json",
            "number,url,title,state,isDraft",
        ],
    )
    .await
    .ok()?;
    if !o.ok {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&o.stdout).ok()?;
    let draft = v
        .get("isDraft")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let state = v.get("state").and_then(serde_json::Value::as_str).map(|s| {
        if draft && s == "OPEN" {
            "draft".to_owned()
        } else {
            s.to_lowercase()
        }
    });
    Some(PullRequestRef {
        number: v.get("number").and_then(serde_json::Value::as_u64)?,
        url: v.get("url").and_then(serde_json::Value::as_str)?.to_owned(),
        title: v
            .get("title")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        state,
    })
}

/// A clone of one of the project's other repositories.
pub struct RepoClone {
    pub repo: RepoRef,
    pub access: RepoAccess,
    pub dir: PathBuf,
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}

/// The clones of the project's other repositories under `~/repos`
/// (`<owner>/<name>`, or `<name>`), told apart by their `origin`.
pub async fn clones(p: &Paths, spec: &SessionSpec) -> Vec<RepoClone> {
    let grants: Vec<RepoGrant> = spec
        .project
        .repos
        .iter()
        .flatten()
        .filter(|r| r.unavailable.is_none())
        .map(|r| r.grant())
        .collect();
    if grants.is_empty() {
        return Vec::new();
    }
    let mut dirs = Vec::new();
    for dir in subdirs(&p.repos) {
        if dir.join(".git").exists() {
            dirs.push(dir);
        } else {
            dirs.extend(
                subdirs(&dir)
                    .into_iter()
                    .filter(|d| d.join(".git").exists()),
            );
        }
    }
    let mut out: Vec<RepoClone> = Vec::new();
    for dir in dirs {
        // The configured URL, before any `insteadOf` rewriting.
        let Ok(url) = git_ok_in(&dir, &p.home, &["config", "--get", "remote.origin.url"]).await
        else {
            continue;
        };
        let Some(repo) = repos::parse_remote(&url) else {
            continue;
        };
        let primary = spec.project.repo.as_ref().is_some_and(|r| r.same(&repo));
        let seen = out.iter().any(|c| c.repo.same(&repo));
        let Some(access) = repos::access(None, &grants, &repo) else {
            continue;
        };
        if primary || seen {
            continue;
        }
        out.push(RepoClone { repo, access, dir });
        if out.len() >= MAX_CLONES {
            break;
        }
    }
    out
}

/// The branch `origin/HEAD` points at: the repository's default branch.
pub async fn default_branch(dir: &Path, home: &Path) -> Option<String> {
    let head = [
        "symbolic-ref",
        "--quiet",
        "--short",
        "refs/remotes/origin/HEAD",
    ];
    if let Ok(s) = git_ok_in(dir, home, &head).await
        && let Some(b) = s.trim().strip_prefix("origin/")
    {
        return Some(b.to_owned());
    }
    for b in ["main", "master"] {
        let remote = format!("refs/remotes/origin/{b}");
        let found = run(
            dir,
            home,
            "git",
            &["rev-parse", "--verify", "--quiet", &remote],
        )
        .await;
        if found.is_ok_and(|o| o.ok) {
            return Some(b.to_owned());
        }
    }
    None
}

/// Keeps work in a writable clone on the session branch: when the agent
/// changed or committed on the default branch, the session branch starts
/// there, with the work, and the default branch goes back to the remote's.
pub async fn onto_session_branch(
    dir: &Path,
    home: &Path,
    branch: &str,
    base: &str,
    st: &GitState,
) -> Result<bool> {
    if st.branch.as_deref() != Some(base) || (st.ahead == 0 && st.uncommitted == 0) {
        return Ok(false);
    }
    let local = format!("refs/heads/{branch}");
    if run(
        dir,
        home,
        "git",
        &["rev-parse", "--verify", "--quiet", &local],
    )
    .await?
    .ok
    {
        return Ok(false);
    }
    git_ok_in(dir, home, &["checkout", "-q", "-b", branch]).await?;
    if st.ahead > 0 {
        git_ok_in(
            dir,
            home,
            &["branch", "-f", base, &format!("origin/{base}")],
        )
        .await?;
    }
    Ok(true)
}

/// What the repositories hold after a turn, and what fctl-agent did about it.
pub struct Synced {
    pub state: GitState,
    /// For the transcript: pushes, and what went wrong.
    pub logs: Vec<(LogLevel, String)>,
}

/// After a turn and when the session stops: pushes new commits on the
/// session branch of the primary repository and of the writable clones,
/// and (with `prs`) looks up their pull requests.
pub async fn sync(p: &Paths, spec: &SessionSpec, push_commits: bool, prs: bool) -> Result<Synced> {
    let mut logs = Vec::new();
    let mut st = state(p, spec).await?;
    if spec.project.repo.is_some() && push_commits && st.ahead > 0 && !st.pushed {
        match push(&p.workdir, &p.home, &st).await {
            Ok(true) => {
                let branch = st.branch.clone().unwrap_or_default();
                logs.push((LogLevel::Info, format!("Pushed {branch}")));
                if let Ok(fresh) = state(p, spec).await {
                    st = fresh;
                }
            }
            Ok(false) => {}
            Err(e) => logs.push((LogLevel::Warn, format!("{e:#}"))),
        }
    }
    if prs
        && st.pushed
        && let Some(b) = st.branch.clone()
    {
        st.pull_request = pull_request(&p.workdir, &p.home, &b).await;
    }
    let mut repos = Vec::new();
    for c in clones(p, spec).await {
        repos.push(sync_clone(p, spec, &c, push_commits, prs, &mut logs).await);
    }
    if !repos.is_empty() {
        st.repos = Some(repos);
    }
    Ok(Synced { state: st, logs })
}

async fn sync_clone(
    p: &Paths,
    spec: &SessionSpec,
    c: &RepoClone,
    push_commits: bool,
    prs: bool,
    logs: &mut Vec<(LogLevel, String)>,
) -> RepoGitState {
    let (dir, home, branch) = (c.dir.as_path(), p.home.as_path(), spec.branch.as_str());
    let name = c.repo.full_name();
    let base = default_branch(dir, home)
        .await
        .unwrap_or_else(|| "main".to_owned());
    let mut st = state_at(dir, home, &base).await.unwrap_or_default();
    if c.access == RepoAccess::Write {
        match onto_session_branch(dir, home, branch, &base, &st).await {
            Ok(true) => {
                logs.push((
                    LogLevel::Info,
                    format!("Moved the work in {name} from {base} to {branch}"),
                ));
                st = state_at(dir, home, &base).await.unwrap_or_default();
            }
            Ok(false) => {}
            Err(e) => logs.push((LogLevel::Warn, format!("{name}: {e:#}"))),
        }
        let on_branch = st.branch.as_deref() == Some(branch);
        if on_branch && push_commits && st.ahead > 0 && !st.pushed {
            match push(dir, home, &st).await {
                Ok(true) => {
                    logs.push((LogLevel::Info, format!("Pushed {branch} to {name}")));
                    if let Ok(fresh) = state_at(dir, home, &base).await {
                        st = fresh;
                    }
                }
                Ok(false) => {}
                Err(e) => logs.push((LogLevel::Warn, format!("{name}: {e:#}"))),
            }
        }
        if prs && on_branch && st.pushed {
            st.pull_request = pull_request(dir, home, branch).await;
        }
    }
    st.files.truncate(MAX_CLONE_FILES);
    let path = match dir.strip_prefix(home) {
        Ok(rel) => format!("~/{}", rel.display()),
        Err(_) => dir.display().to_string(),
    };
    RepoGitState::new(&c.repo, c.access, Some(path), st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    fn spec() -> Result<SessionSpec> {
        spec_with(serde_json::Value::Null)
    }

    /// acme/widgets, with `repos` as the project's other repositories.
    fn spec_with(repos: serde_json::Value) -> Result<SessionSpec> {
        let mut spec = serde_json::json!({
            "session_id": "ses_test",
            "number": 1,
            "title": "Test",
            "project": {
                "id": "prj_test",
                "slug": "widgets",
                "name": "Widgets",
                "repo": { "owner": "acme", "name": "widgets" },
                "default_branch": "main",
                "settings": {}
            },
            "branch": "fctl/1-test",
            "base_branch": "main",
            "author": { "id": "usr_test", "name": "Ada" },
            "claude_session_id": "00000000-0000-0000-0000-000000000000",
            "resume": false,
            "secrets": {},
            "credentials": {
                "claude": {
                    "id": "crd_test",
                    "kind": "setup_token",
                    "access_token": "test",
                    "scopes": []
                }
            },
            "git_author": { "name": "fctl", "email": "fctl@example.com" },
            "co_authors": [],
            "web_url": "https://fctl.test/s/ses_test",
            "idle_timeout_secs": 600
        });
        if !repos.is_null() {
            spec["project"]["repos"] = repos;
        }
        Ok(serde_json::from_value(spec)?)
    }

    fn sandbox(dir: &Path) -> Result<Paths> {
        let p = Paths::new(
            &dir.join("workspace"),
            &dir.join("home"),
            &dir.join("state"),
        );
        std::fs::create_dir_all(&p.home)?;
        Ok(p)
    }

    /// A local repository standing in for `https://github.com/{full}.git`.
    async fn fake_github(dir: &Path, home: &Path, full: &str) -> Result<PathBuf> {
        let origin = dir.join("github").join(full);
        std::fs::create_dir_all(&origin)?;
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["commit", "-q", "--allow-empty", "-m", "Initial commit"],
        ] {
            assert!(run(&origin, home, "git", args).await?.ok);
        }
        let rewrite = format!("url.{}.insteadOf", origin.display());
        let url = format!("https://github.com/{full}.git");
        let config = ["config", "--global", rewrite.as_str(), url.as_str()];
        assert!(run(home, home, "git", &config).await?.ok);
        Ok(origin)
    }

    /// The password git's credential helpers hand out for a path on github.com.
    async fn token_for(home: &Path, path: Option<&str>) -> Result<Option<String>> {
        let mut input = String::from("protocol=https\nhost=github.com\n");
        if let Some(path) = path {
            input.push_str(&format!("path={path}\n"));
        }
        input.push('\n');
        let mut child = Command::new("git")
            .args(["credential", "fill"])
            .current_dir(home)
            .env("HOME", home)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdin = child.stdin.take().context("stdin")?;
        stdin.write_all(input.as_bytes()).await?;
        drop(stdin);
        let out = child.wait_with_output().await?;
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|l| l.strip_prefix("password="))
            .map(str::to_owned))
    }

    fn cred(token: &str) -> GithubCredential {
        GithubCredential {
            token: token.to_owned(),
            expires_at: 0,
        }
    }

    #[tokio::test]
    async fn plain_push_publishes_a_new_branch_under_its_own_name() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let p = Paths::new(
            &dir.path().join("workspace"),
            &dir.path().join("home"),
            &dir.path().join("state"),
        );
        std::fs::create_dir_all(&p.home)?;
        let spec = spec()?;
        configure_git(&p, &spec.git_author).await?;
        // A local repository stands in for GitHub.
        let origin = dir.path().join("origin");
        std::fs::create_dir_all(&origin)?;
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["commit", "-q", "--allow-empty", "-m", "Initial commit"],
        ] {
            assert!(run(&origin, &p.home, "git", args).await?.ok);
        }
        let clone_url = spec.project.repo.as_ref().context("repo")?.clone_url();
        let rewrite = format!("url.{}.insteadOf", origin.display());
        let config = ["config", "--global", rewrite.as_str(), clone_url.as_str()];
        assert!(run(&p.home, &p.home, "git", &config).await?.ok);

        let (log, _logs) = mpsc::unbounded_channel();
        prepare(&p, &spec, &log).await?;
        let push = git(&p, &["push", "--quiet"]).await?;
        assert!(push.ok, "git push failed: {}", push.stderr);
        let upstream = git_ok(&p, &["rev-parse", "--abbrev-ref", "@{upstream}"]).await?;
        assert_eq!(upstream.trim(), "origin/fctl/1-test");
        let pushed = ["rev-parse", "--verify", "--quiet", "refs/heads/fctl/1-test"];
        assert!(run(&origin, &p.home, "git", &pushed).await?.ok);
        Ok(())
    }

    #[tokio::test]
    async fn git_gets_the_token_for_each_repository() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let p = sandbox(dir.path())?;
        let spec = spec_with(serde_json::json!([
            { "owner": "acme", "name": "lib", "access": "write" },
            { "owner": "acme", "name": "docs", "access": "read" },
            { "owner": "acme", "access": "read" }
        ]))?;
        configure_git(&p, &spec.git_author).await?;
        write_routes(&p, &spec)?;
        write_github_tokens(&p, Some(&cred("ghs_write")), Some(&cred("ghs_read")))?;
        let token = |path: Option<&'static str>| token_for(&p.home, path);
        let write = Some("ghs_write".to_owned());
        let read = Some("ghs_read".to_owned());
        assert_eq!(token(Some("acme/widgets.git")).await?, write);
        assert_eq!(token(Some("Acme/Lib")).await?, write);
        assert_eq!(token(Some("acme/docs.git")).await?, read);
        assert_eq!(token(Some("acme/design.git/info/lfs")).await?, read);
        // Without a path, the token that can write, as before.
        assert_eq!(token(None).await?, write);
        let hosts = std::fs::read_to_string(p.home.join(".config/gh/hosts.yml"))?;
        assert!(hosts.contains("oauth_token: ghs_write"));
        // A refresh replaces both, and a second configure adds no second helper.
        write_github_tokens(&p, Some(&cred("ghs_write2")), Some(&cred("ghs_read2")))?;
        configure_git(&p, &spec.git_author).await?;
        assert_eq!(
            token(Some("acme/lib.git")).await?,
            Some("ghs_write2".into())
        );
        assert_eq!(
            token(Some("acme/docs.git")).await?,
            Some("ghs_read2".into())
        );
        let helpers = [
            "config",
            "--global",
            "--get-all",
            "credential.https://github.com.helper",
        ];
        let listed = git_ok_in(&p.home, &p.home, &helpers).await?;
        assert_eq!(listed.lines().filter(|l| !l.is_empty()).count(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn a_session_with_one_kind_of_token_uses_it_everywhere() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let p = sandbox(dir.path())?;
        configure_git(&p, &spec()?.git_author).await?;
        write_routes(&p, &spec()?)?;
        write_github_tokens(&p, None, Some(&cred("ghs_read")))?;
        assert_eq!(
            token_for(&p.home, Some("acme/widgets.git")).await?,
            Some("ghs_read".into())
        );
        let hosts = std::fs::read_to_string(p.home.join(".config/gh/hosts.yml"))?;
        assert!(hosts.contains("oauth_token: ghs_read"));
        Ok(())
    }

    #[tokio::test]
    async fn work_in_other_repositories_lands_on_the_session_branch() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let p = sandbox(dir.path())?;
        let spec = spec_with(serde_json::json!([
            { "owner": "acme", "name": "lib", "access": "write" },
            { "owner": "acme", "name": "docs", "access": "read" }
        ]))?;
        configure_git(&p, &spec.git_author).await?;
        fake_github(dir.path(), &p.home, "acme/widgets").await?;
        let lib = fake_github(dir.path(), &p.home, "acme/lib").await?;
        let docs = fake_github(dir.path(), &p.home, "acme/docs").await?;
        fake_github(dir.path(), &p.home, "acme/other").await?;
        let (log, _logs) = mpsc::unbounded_channel();
        prepare(&p, &spec, &log).await?;
        // The agent clones what it needs where it was told to, and also the
        // primary repository and one it may not use.
        for full in ["acme/lib", "acme/docs", "acme/other", "acme/widgets"] {
            let url = format!("https://github.com/{full}.git");
            let to = p.repos.join(full).display().to_string();
            assert!(
                run(&p.home, &p.home, "git", &["clone", "-q", &url, &to])
                    .await?
                    .ok
            );
        }
        // It commits on the default branches.
        for full in ["acme/lib", "acme/docs"] {
            let clone = p.repos.join(full);
            std::fs::write(clone.join("change.txt"), "hello\n")?;
            assert!(
                run(&clone, &p.home, "git", &["add", "change.txt"])
                    .await?
                    .ok
            );
            let commit = ["commit", "-q", "-m", "Change"];
            assert!(run(&clone, &p.home, "git", &commit).await?.ok);
        }

        let synced = sync(&p, &spec, true, false).await?;
        let repos = synced.state.repos.context("repos")?;
        let names: Vec<String> = repos.iter().map(RepoGitState::full_name).collect();
        assert_eq!(names, ["acme/docs", "acme/lib"]);
        let session_branch = "refs/heads/fctl/1-test";
        let has_branch = ["rev-parse", "--verify", "--quiet", session_branch];

        // Writable: moved to the session branch and pushed; main as on GitHub.
        let l = &repos[1];
        assert_eq!(l.access, RepoAccess::Write);
        assert_eq!(l.branch.as_deref(), Some("fctl/1-test"));
        assert_eq!(l.base.as_deref(), Some("main"));
        assert_eq!((l.ahead, l.pushed), (1, true));
        assert_eq!(l.files.len(), 1);
        assert_eq!(l.path.as_deref(), Some("~/repos/acme/lib"));
        assert!(run(&lib, &p.home, "git", &has_branch).await?.ok);
        let local_main = ["rev-list", "--count", "origin/main..main"];
        let ahead = git_ok_in(&p.repos.join("acme/lib"), &p.home, &local_main).await?;
        assert_eq!(ahead.trim(), "0");
        assert!(
            synced
                .logs
                .iter()
                .any(|(_, t)| t == "Pushed fctl/1-test to acme/lib")
        );

        // Read-only: left alone, nothing pushed.
        let d = &repos[0];
        assert_eq!(d.access, RepoAccess::Read);
        assert_eq!(d.branch.as_deref(), Some("main"));
        assert_eq!((d.ahead, d.pushed), (1, false));
        assert!(!run(&docs, &p.home, "git", &has_branch).await?.ok);

        // The next turn changes nothing more.
        let again = sync(&p, &spec, true, false).await?;
        assert!(again.logs.is_empty(), "{:?}", again.logs);
        Ok(())
    }
}
