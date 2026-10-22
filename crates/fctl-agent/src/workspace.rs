//! The session's repositories: cloning the primary one, branches, the setup
//! script, their state while Claude works and after each turn, pushing, and
//! the credentials git and `gh` use.
//!
//! The agent clones the project's other repositories itself, when it needs
//! them, into `~/repos/<owner>/<name>`. fctl-agent finds those clones each
//! time it reports the state; after each turn it also keeps the work in
//! writable ones on the session branch, pushes it, and looks up its pull
//! request. A branch whose pull request merged isn't pushed again.

use crate::followed::{self, Pulls};
use anyhow::{Context, Result, bail};
use fctl_domain::repos;
use fctl_proto::event::{CommitInfo, FileChange, GitState, LogLevel, RepoGitState};
use fctl_proto::runner::{GitIdentity, GithubCredential, SessionSpec};
use fctl_proto::session::{GithubAccess, RepoAccess, RepoGrant, RepoRef};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// Where attachments are saved: `~/attachments`, outside the repository.
    pub attachments: PathBuf,
    /// The GitHub token that can write: the primary repository and the
    /// other writable ones.
    pub token_file: PathBuf,
    /// The read-only GitHub token.
    pub read_token_file: PathBuf,
    /// The repositories `token_file` covers: `owner/name`, lowercase, one per line.
    pub write_repos_file: PathBuf,
    pub helper: PathBuf,
    /// The `gh` that looks up pull requests: the sandbox's, a stand-in in
    /// tests.
    pub gh: String,
}

impl Paths {
    pub fn new(workdir: &Path, home: &Path, state: &Path) -> Self {
        Self {
            workdir: workdir.to_path_buf(),
            home: home.to_path_buf(),
            repos: home.join("repos"),
            attachments: home.join("attachments"),
            token_file: state.join("github-token"),
            read_token_file: state.join("github-token-read"),
            write_repos_file: state.join("github-write-repos"),
            helper: state.join("git-credential-fctl"),
            gh: "gh".to_owned(),
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
    run_with(dir, home, program, args, &[]).await
}

/// [`run`], with more environment variables. The command is killed when
/// its caller stops waiting for it (a timeout).
async fn run_with(
    dir: &Path,
    home: &Path,
    program: &str,
    args: &[&str],
    env: &[(&str, &OsStr)],
) -> Result<Output> {
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("GIT_TERMINAL_PROMPT", "0")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .kill_on_drop(true)
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
/// is one. A session whose agent only reads has none, and its read-only
/// token covers every repository: an earlier one that could write, from
/// before its project held its agents to reading, goes.
pub fn write_github_tokens(
    p: &Paths,
    write: Option<&GithubCredential>,
    read: Option<&GithubCredential>,
) -> Result<()> {
    if let Some(dir) = p.token_file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    match write {
        Some(cred) => write_private(&p.token_file, &cred.token)?,
        None if read.is_some() => {
            let _ = std::fs::remove_file(&p.token_file);
        }
        None => {}
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

/// A private copy of a clone's index, for git commands that only read it
/// but may write it: `git diff` refreshes the index it reads when files'
/// stat data is stale, holding the index lock meanwhile, which would make
/// the agent's own `git add` or `git commit` fail if they ran at that
/// moment, as they can while Claude works. `GIT_OPTIONAL_LOCKS` keeps
/// `git status` from doing the same, not `git diff`.
struct IndexCopy(Option<PathBuf>);

impl IndexCopy {
    async fn of(dir: &Path, home: &Path) -> Self {
        static COPIES: AtomicU64 = AtomicU64::new(0);
        let Ok(index) = git_ok_in(dir, home, &["rev-parse", "--git-path", "index"]).await else {
            return Self(None);
        };
        let n = COPIES.fetch_add(1, Ordering::Relaxed);
        let copy = std::env::temp_dir().join(format!("fctl-index-{}-{n}", std::process::id()));
        // Without an index (nothing staged yet) there is nothing to write,
        // and without a copy git reads the index itself.
        match std::fs::copy(dir.join(index.trim()), &copy) {
            Ok(_) => Self(Some(copy)),
            Err(_) => Self(None),
        }
    }

    /// The environment that has git read the copy, and skip optional locks.
    fn env(&self) -> Vec<(&'static str, &OsStr)> {
        let mut env = vec![("GIT_OPTIONAL_LOCKS", OsStr::new("0"))];
        if let Some(copy) = &self.0 {
            env.push(("GIT_INDEX_FILE", copy.as_os_str()));
        }
        env
    }
}

impl Drop for IndexCopy {
    fn drop(&mut self) {
        if let Some(copy) = &self.0 {
            let _ = std::fs::remove_file(copy);
        }
    }
}

/// The primary repository's state relative to the base branch; `pulls`
/// tells whether its branch's pull request merged.
pub async fn state(p: &Paths, spec: &SessionSpec, pulls: &Pulls) -> Result<GitState> {
    let followed = spec.project.repo.as_ref().map(|r| (r, pulls));
    state_at(&p.workdir, &p.home, &spec.base_branch, followed).await
}

/// A clone's state relative to `base`, a branch of its `origin`. With the
/// repository it is of, the pull requests fctl-agent follows tell whether
/// its branch's merged: then its changes are the work that merged and
/// what came after.
pub async fn state_at(
    dir: &Path,
    home: &Path,
    base: &str,
    followed: Option<(&RepoRef, &Pulls)>,
) -> Result<GitState> {
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
    let merged = followed.and_then(|(repo, pulls)| pulls.merged(Some(repo), branch.as_deref()));
    let index = IndexCopy::of(dir, home).await;
    let env = index.env();
    let mut files: Vec<FileChange> = Vec::new();
    // What the branch changed since it left the base: the diff from their
    // merge base, which the base's later commits don't move. Once its pull
    // request merged, from the base as it was just before (`baseRefOid`):
    // a merge commit puts the branch's commits on the base, which would
    // leave nothing to compare. Without a base, or a merge base with it,
    // everything is new: the diff from the empty tree.
    const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
    let mut diff_target = EMPTY_TREE.to_owned();
    if has_base && head.is_some() {
        let before_merge = merged.and_then(|f| f.base.as_deref());
        for from in before_merge.into_iter().chain([base_ref.as_str()]) {
            if let Ok(fork) = git_ok_in(dir, home, &["merge-base", from, "HEAD"]).await {
                diff_target = fork.trim().to_owned();
                break;
            }
        }
    }
    if head.is_some() {
        let numstat = ["diff", "--numstat", "-M", &diff_target];
        let numstat = run_with(dir, home, "git", &numstat, &env).await?;
        let names = ["diff", "--name-status", "-M", &diff_target];
        let names = run_with(dir, home, "git", &names, &env).await?;
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
    let porcelain = run_with(dir, home, "git", &["status", "--porcelain"], &env).await?;
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
    // The commits after the head that merged, which the base doesn't have.
    let unmerged = match (merged.and_then(|f| f.head.as_deref()), &head) {
        (Some(merged_head), Some(_)) => {
            let mut count = vec!["rev-list", "--count", "HEAD", "--not", merged_head];
            if has_base {
                count.push(&base_ref);
            }
            git_ok_in(dir, home, &count)
                .await
                .ok()
                .and_then(|s| s.trim().parse().ok())
        }
        _ => None,
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
        unmerged,
        repos: None,
        pull_requests: None,
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

/// Whether `branch` was pushed and GitHub no longer has it: its upstream
/// is set, but a fetch pruned it from the clone (`[gone]`), as when GitHub
/// deleted the branch after its pull request merged.
async fn upstream_gone(dir: &Path, home: &Path, branch: &str) -> bool {
    let branch = format!("refs/heads/{branch}");
    let track = [
        "for-each-ref",
        "--format=%(upstream:track)",
        branch.as_str(),
    ];
    git_ok_in(dir, home, &track)
        .await
        .is_ok_and(|s| s.trim() == "[gone]")
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
        // An agent that doesn't push writes to none of them.
        let access = if pushes(spec) {
            access
        } else {
            RepoAccess::Read
        };
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

/// When fctl-agent reports the repositories' state, which decides what else
/// it does: it changes them (moves work onto the session branch, pushes)
/// only once a turn has ended or as the session stops, and asks GitHub for
/// pull requests only where it can find one it hasn't, or one can have
/// changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moment {
    /// While Claude works: it reads them, and looks up the pull request of a
    /// branch once it was pushed or its pushed head moved since the last
    /// report, or after `gh pr create` ran (`opened_pr`), the pull requests
    /// that printed too.
    Working { opened_pr: bool },
    /// A turn ended: the work in writable clones moves onto the session
    /// branch, new commits are pushed, unless their branch's pull request
    /// merged, and every pull request is looked up that isn't merged.
    TurnEnded,
    /// The session stops: as when a turn ends, without lookups.
    Stopping,
}

impl Moment {
    /// Whether fctl-agent may change the repositories: move work onto the
    /// session branch and push it.
    fn writes(self) -> bool {
        !matches!(self, Moment::Working { .. })
    }

    /// Whether to look up the pull request of a branch whose head is on the
    /// remote as `now` says. `before` is how the last report had it: `None`
    /// when it didn't have the repository.
    fn looks_up(self, now: OnRemote, before: Option<OnRemote>) -> bool {
        now.is_some()
            && match self {
                Moment::Working { opened_pr } => opened_pr || before != Some(now),
                Moment::TurnEnded => true,
                Moment::Stopping => false,
            }
    }

    /// Whether to look up the pull request of a branch that GitHub deleted
    /// after it was pushed, unless it is known to have merged: once after
    /// the agent starts, and after `gh pr create` or a turn.
    fn looks_up_deleted(self, before: Option<OnRemote>) -> bool {
        match self {
            Moment::Working { opened_pr } => opened_pr || before.is_none(),
            Moment::TurnEnded => true,
            Moment::Stopping => false,
        }
    }
}

/// A branch and its head, when the remote has that commit.
type OnRemote<'a> = Option<(&'a str, &'a str)>;

fn on_remote<'a>(
    branch: &'a Option<String>,
    head: &'a Option<String>,
    pushed: bool,
) -> OnRemote<'a> {
    match (branch, head) {
        (Some(b), Some(h)) if pushed => Some((b, h)),
        _ => None,
    }
}

/// How the last report had a repository's branch.
#[derive(Clone, Copy)]
struct Before<'a> {
    branch: &'a Option<String>,
    head: &'a Option<String>,
    pushed: bool,
}

impl<'a> Before<'a> {
    fn of(st: &'a GitState) -> Self {
        Self {
            branch: &st.branch,
            head: &st.head,
            pushed: st.pushed,
        }
    }

    fn of_repo(r: &'a RepoGitState) -> Self {
        Self {
            branch: &r.branch,
            head: &r.head,
            pushed: r.pushed,
        }
    }

    fn on_remote(self) -> OnRemote<'a> {
        on_remote(self.branch, self.head, self.pushed)
    }
}

/// What the repositories hold, and what fctl-agent did about it.
pub struct Synced {
    pub state: GitState,
    /// For the transcript: pushes, and what went wrong.
    pub logs: Vec<(LogLevel, String)>,
}

/// A sync of the repositories as it goes through them.
struct Run<'a> {
    p: &'a Paths,
    moment: Moment,
    pulls: &'a mut Pulls,
    /// The URLs of the pull requests looked up so far.
    looked: Vec<String>,
    logs: Vec<(LogLevel, String)>,
}

/// A repository whose branch fctl-agent pushes and looks up the pull
/// request of: the primary one, or a writable clone on the session branch.
struct Writable<'a> {
    dir: &'a Path,
    repo: &'a RepoRef,
    /// The branch its state is relative to.
    base: &'a str,
    /// `owner/name`, for the others.
    name: Option<String>,
}

impl Run<'_> {
    async fn state(&self, w: &Writable<'_>) -> Result<GitState> {
        state_at(w.dir, &self.p.home, w.base, Some((w.repo, &*self.pulls))).await
    }

    /// Looks up the pull request of `branch`, checked out in `w`; if what it
    /// finds changes whether that merged, or how, `st` follows.
    async fn look_up(&mut self, w: &Writable<'_>, st: &mut GitState, branch: &str) {
        let Some(found) = followed::look_up(self.p, w.dir, branch).await else {
            return;
        };
        let was = self.pulls.merged(Some(w.repo), Some(branch)).cloned();
        self.looked.push(found.pr.url.clone());
        self.pulls.found(found);
        if self.pulls.merged(Some(w.repo), Some(branch)) != was.as_ref()
            && let Ok(fresh) = self.state(w).await
        {
            *st = fresh;
        }
    }

    /// Pushes the new commits on `w`'s branch when `push` says fctl-agent
    /// may, unless the branch's pull request merged: GitHub has what it
    /// merged, and a push would bring back a branch it deleted, so the
    /// commits after it stay in the sandbox, and the transcript says so.
    /// Then looks up the pull request where one can have appeared or
    /// changed. `before` is how the last report had the branch.
    async fn publish(
        &mut self,
        w: &Writable<'_>,
        st: &mut GitState,
        push: bool,
        before: Option<Before<'_>>,
    ) {
        let Some(branch) = st.branch.clone().filter(|b| b != "HEAD") else {
            return;
        };
        if st.head.is_none() {
            return;
        }
        let home = self.p.home.as_path();
        let to = w
            .name
            .as_ref()
            .map(|n| format!(" to {n}"))
            .unwrap_or_default();
        let warn = |e: anyhow::Error| match &w.name {
            Some(n) => format!("{n}: {e:#}"),
            None => format!("{e:#}"),
        };
        // Whether GitHub deleted the branch, asked of git once.
        let mut gone = None;
        let mut looked_up = false;
        if push && st.ahead > 0 && !st.pushed {
            // A branch whose pull request was open, or that GitHub deleted,
            // can have merged since the last lookup: ask before pushing.
            let known = self.pulls.of_branch(w.repo, &branch).map(|f| f.merged());
            if self.moment == Moment::TurnEnded && known != Some(true) {
                let deleted = known.is_none() && upstream_gone(w.dir, home, &branch).await;
                gone = Some(deleted);
                if known.is_some() || deleted {
                    self.look_up(w, st, &branch).await;
                    looked_up = true;
                }
            }
            match self.pulls.merged(Some(w.repo), Some(&branch)) {
                Some(f) => {
                    let moved = before.is_none_or(|b| *b.head != st.head);
                    if st.unmerged != Some(0) && moved {
                        let number = f.pr.number;
                        let text = format!(
                            "Didn't push {branch}{to}: its pull request #{number} is merged"
                        );
                        self.logs.push((LogLevel::Info, text));
                    }
                }
                None => match self::push(w.dir, home, st).await {
                    Ok(true) => {
                        self.logs
                            .push((LogLevel::Info, format!("Pushed {branch}{to}")));
                        if let Ok(fresh) = self.state(w).await {
                            *st = fresh;
                        }
                    }
                    Ok(false) => {}
                    Err(e) => self.logs.push((LogLevel::Warn, warn(e))),
                },
            }
        }
        if looked_up {
            return;
        }
        let now = on_remote(&st.branch, &st.head, st.pushed);
        let before = before.map(Before::on_remote);
        let mut look = self.moment.looks_up(now, before);
        if !look
            && now.is_none()
            && self.moment.looks_up_deleted(before)
            && self.pulls.merged(Some(w.repo), Some(&branch)).is_none()
        {
            look = match gone {
                Some(deleted) => deleted,
                None => upstream_gone(w.dir, home, &branch).await,
            };
        }
        if look {
            self.look_up(w, st, &branch).await;
        }
    }

    /// Gives `st`, read before the last lookups, its branch's pull request
    /// as they left it; `was`, the pull requests before them, tells whether
    /// they changed what merged.
    async fn attach(&self, w: &Writable<'_>, st: &mut GitState, was: &Pulls) {
        let branch = st.branch.as_deref();
        if self.pulls.merged(Some(w.repo), branch) != was.merged(Some(w.repo), branch)
            && let Ok(fresh) = self.state(w).await
        {
            *st = fresh;
        }
        st.pull_request = st
            .branch
            .as_deref()
            .and_then(|b| self.pulls.of_branch(w.repo, b))
            .map(|f| f.pr.clone());
    }
}

/// Whether the session's agent pushes: unless it may only comment or read
/// on GitHub, whose tokens can't push.
pub fn pushes(spec: &SessionSpec) -> bool {
    spec.github_access.is_none_or(GithubAccess::pushes)
}

/// The state of the primary repository and the clones of the others at
/// `moment`, with the pull requests fctl-agent follows (`pulls`), which
/// lookups update. When a turn ends or the session stops it also moves the
/// work in writable clones onto the session branch and, with
/// `push_commits`, pushes new commits on the session branch. `last` is the
/// state reported last. A session whose agent doesn't push ([`pushes`])
/// moves and pushes nothing.
pub async fn sync(
    p: &Paths,
    spec: &SessionSpec,
    push_commits: bool,
    moment: Moment,
    last: Option<&GitState>,
    pulls: &mut Pulls,
) -> Result<Synced> {
    let push_commits = push_commits && pushes(spec);
    let mut run = Run {
        p,
        moment,
        pulls,
        looked: Vec::new(),
        logs: Vec::new(),
    };
    let mut st = state(p, spec, run.pulls).await?;
    let primary = spec.project.repo.as_ref().map(|repo| Writable {
        dir: &p.workdir,
        repo,
        base: &spec.base_branch,
        name: None,
    });
    if let Some(w) = &primary {
        let push = moment.writes() && push_commits;
        run.publish(w, &mut st, push, last.map(Before::of)).await;
    }
    let mut others = Vec::new();
    for c in clones(p, spec).await {
        let before = last.and_then(|l| l.repos.as_ref()).and_then(|rs| {
            rs.iter()
                .find(|r| r.owner == c.repo.owner && r.name == c.repo.name)
                .map(Before::of_repo)
        });
        let (base, cst) = sync_clone(&mut run, spec, &c, push_commits, before).await;
        others.push((c, base, cst));
    }
    // The pull requests of other branches, worktrees and clones.
    let was = run.pulls.clone();
    for url in run.pulls.due(moment, &run.looked) {
        if let Some(found) = followed::look_up(p, &p.home, &url).await {
            run.pulls.found(found);
        }
    }
    if let Some(w) = &primary {
        run.attach(w, &mut st, &was).await;
    }
    let mut repos = Vec::new();
    for (c, base, mut cst) in others {
        let w = Writable {
            dir: &c.dir,
            repo: &c.repo,
            base: &base,
            name: None,
        };
        run.attach(&w, &mut cst, &was).await;
        cst.files.truncate(MAX_CLONE_FILES);
        let path = match c.dir.strip_prefix(&p.home) {
            Ok(rel) => format!("~/{}", rel.display()),
            Err(_) => c.dir.display().to_string(),
        };
        repos.push(RepoGitState::new(&c.repo, c.access, Some(path), cst));
    }
    if !repos.is_empty() {
        st.repos = Some(repos);
    }
    let listed = run.pulls.listed();
    if !listed.is_empty() {
        st.pull_requests = Some(listed);
    }
    Ok(Synced {
        state: st,
        logs: run.logs,
    })
}

/// A clone of another repository: in a writable one, the work moves onto
/// the session branch, which fctl-agent pushes and looks up the pull
/// request of. Returns its base branch and its state.
async fn sync_clone(
    run: &mut Run<'_>,
    spec: &SessionSpec,
    c: &RepoClone,
    push_commits: bool,
    before: Option<Before<'_>>,
) -> (String, GitState) {
    let (dir, home, branch) = (c.dir.as_path(), run.p.home.as_path(), spec.branch.as_str());
    let name = c.repo.full_name();
    let base = default_branch(dir, home)
        .await
        .unwrap_or_else(|| "main".to_owned());
    let w = Writable {
        dir,
        repo: &c.repo,
        base: &base,
        name: Some(name.clone()),
    };
    let mut st = run.state(&w).await.unwrap_or_default();
    if c.access == RepoAccess::Write {
        if run.moment.writes() {
            match onto_session_branch(dir, home, branch, &base, &st).await {
                Ok(true) => {
                    run.logs.push((
                        LogLevel::Info,
                        format!("Moved the work in {name} from {base} to {branch}"),
                    ));
                    st = run.state(&w).await.unwrap_or_default();
                }
                Ok(false) => {}
                Err(e) => run.logs.push((LogLevel::Warn, format!("{name}: {e:#}"))),
            }
        }
        if st.branch.as_deref() == Some(branch) {
            let push = run.moment.writes() && push_commits;
            run.publish(&w, &mut st, push, before).await;
        }
    }
    (base, st)
}

/// Whether a repository's HEAD is detached, as in the middle of a rebase.
pub fn detached(st: &GitState) -> bool {
    let detached = |branch: &Option<String>| branch.as_deref() == Some("HEAD");
    detached(&st.branch) || st.repos.iter().flatten().any(|r| detached(&r.branch))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    pub(crate) fn spec() -> Result<SessionSpec> {
        spec_with(serde_json::Value::Null)
    }

    /// acme/widgets, with `repos` as the project's other repositories.
    pub(crate) fn spec_with(repos: serde_json::Value) -> Result<SessionSpec> {
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

    /// The sandbox's paths, with a `gh` that isn't there: tests that look up
    /// pull requests install a [`FakeGh`].
    fn sandbox(dir: &Path) -> Result<Paths> {
        let mut p = Paths::new(
            &dir.join("workspace"),
            &dir.join("home"),
            &dir.join("state"),
        );
        p.gh = dir.join("no-gh").display().to_string();
        std::fs::create_dir_all(&p.home)?;
        Ok(p)
    }

    const FAKE_GH: &str = r#"#!/bin/sh
# A stand-in for gh: logs each call, and answers `gh pr view <branch or URL>`
# with the pull request it was given for it.
here=$(dirname "$0")
echo "$*" >> "$here/calls"
[ "$1 $2" = "pr view" ] || exit 1
case "$*" in
  *baseRefOid*) [ -f "$here/old" ] && { echo 'Unknown JSON field: "baseRefOid"' >&2; exit 1; } ;;
esac
answer="$here/prs/$(printf '%s' "$3" | tr -c 'A-Za-z0-9._-' '_')"
[ -f "$answer" ] || { echo "no pull requests found for branch \"$3\"" >&2; exit 1; }
cat "$answer"
"#;

    /// A stand-in for `gh` that knows the pull requests a test gives it.
    struct FakeGh(PathBuf);

    impl FakeGh {
        fn install(p: &mut Paths, dir: &Path) -> Result<Self> {
            let here = dir.join("fake-gh");
            std::fs::create_dir_all(here.join("prs"))?;
            let gh = here.join("gh");
            std::fs::write(&gh, FAKE_GH)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755))?;
            }
            p.gh = gh.display().to_string();
            Ok(Self(here))
        }

        /// Answers with pull request `number` of acme/widgets, as GitHub has
        /// it now, for its branch and for its URL.
        fn knows(
            &self,
            number: u64,
            branch: &str,
            state: &str,
            head: &str,
            base: &str,
        ) -> Result<()> {
            let url = format!("https://github.com/acme/widgets/pull/{number}");
            let pr = serde_json::json!({
                "number": number, "url": url, "title": format!("Change {number}"),
                "state": state, "isDraft": false,
                "headRefName": branch, "headRefOid": head, "baseRefOid": base
            });
            for key in [branch, url.as_str()] {
                let name: String = key
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || "._-".contains(c) {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                std::fs::write(self.0.join("prs").join(name), pr.to_string())?;
            }
            Ok(())
        }

        /// From now on, as a gh from before `baseRefOid`.
        fn grow_old(&self) -> Result<()> {
            Ok(std::fs::write(self.0.join("old"), "")?)
        }

        /// What it was asked, one call per line.
        fn calls(&self) -> String {
            std::fs::read_to_string(self.0.join("calls")).unwrap_or_default()
        }
    }

    /// The changed files: `status path +additions -deletions`.
    fn changes(st: &GitState) -> Vec<String> {
        st.files
            .iter()
            .map(|f| {
                let n = |c: Option<u32>| c.map_or("?".to_owned(), |c| c.to_string());
                format!(
                    "{} {} +{} -{}",
                    f.status,
                    f.path,
                    n(f.additions),
                    n(f.deletions)
                )
            })
            .collect()
    }

    async fn commit(dir: &Path, home: &Path, file: &str, text: &str, msg: &str) -> Result<String> {
        std::fs::write(dir.join(file), text)?;
        git_ok_in(dir, home, &["add", file]).await?;
        git_ok_in(dir, home, &["commit", "-q", "-m", msg]).await?;
        Ok(git_ok_in(dir, home, &["rev-parse", "HEAD"])
            .await?
            .trim()
            .to_owned())
    }

    /// What GitHub does when it merges the pull request of `branch` and
    /// deletes the branch: squashes it into one commit on main, or merges
    /// it with a merge commit. Returns main's head from before.
    async fn merge_on_github(
        origin: &Path,
        home: &Path,
        branch: &str,
        squash: bool,
    ) -> Result<String> {
        let before = git_ok_in(origin, home, &["rev-parse", "main"]).await?;
        if squash {
            git_ok_in(origin, home, &["merge", "-q", "--squash", branch]).await?;
            git_ok_in(origin, home, &["commit", "-q", "-m", "Change 1 (#1)"]).await?;
        } else {
            let merge = [
                "merge",
                "-q",
                "--no-ff",
                "-m",
                "Merge pull request #1",
                branch,
            ];
            git_ok_in(origin, home, &merge).await?;
        }
        git_ok_in(origin, home, &["branch", "-q", "-D", branch]).await?;
        Ok(before.trim().to_owned())
    }

    async fn has_branch(origin: &Path, home: &Path, branch: &str) -> Result<bool> {
        let r = format!("refs/heads/{branch}");
        Ok(run(
            origin,
            home,
            "git",
            &["rev-parse", "--verify", "--quiet", &r],
        )
        .await?
        .ok)
    }

    fn log_texts(logs: &[(LogLevel, String)]) -> Vec<&str> {
        logs.iter().map(|(_, t)| t.as_str()).collect()
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

        let synced = sync(
            &p,
            &spec,
            true,
            Moment::Stopping,
            None,
            &mut Pulls::default(),
        )
        .await?;
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
        let again = sync(
            &p,
            &spec,
            true,
            Moment::Stopping,
            None,
            &mut Pulls::default(),
        )
        .await?;
        assert!(again.logs.is_empty(), "{:?}", again.logs);
        Ok(())
    }

    #[tokio::test]
    async fn an_agent_that_comments_pushes_and_moves_nothing() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let p = sandbox(dir.path())?;
        let spec = SessionSpec {
            github_access: Some(GithubAccess::Comment),
            ..spec_with(serde_json::json!([
                { "owner": "acme", "name": "lib", "access": "write" }
            ]))?
        };
        assert!(!pushes(&spec));
        configure_git(&p, &spec.git_author).await?;
        let widgets = fake_github(dir.path(), &p.home, "acme/widgets").await?;
        let lib = fake_github(dir.path(), &p.home, "acme/lib").await?;
        let (log, _logs) = mpsc::unbounded_channel();
        prepare(&p, &spec, &log).await?;
        let url = "https://github.com/acme/lib.git";
        let to = p.repos.join("acme/lib").display().to_string();
        assert!(
            run(&p.home, &p.home, "git", &["clone", "-q", url, &to])
                .await?
                .ok
        );
        // It commits on the session branch, and on lib's default branch.
        for dir in [p.workdir.clone(), p.repos.join("acme/lib")] {
            std::fs::write(dir.join("change.txt"), "hello\n")?;
            assert!(run(&dir, &p.home, "git", &["add", "change.txt"]).await?.ok);
            let commit = ["commit", "-q", "-m", "Change"];
            assert!(run(&dir, &p.home, "git", &commit).await?.ok);
        }
        let synced = sync(
            &p,
            &spec,
            true,
            Moment::Stopping,
            None,
            &mut Pulls::default(),
        )
        .await?;
        assert!(
            !synced.logs.iter().any(|(_, t)| t.starts_with("Pushed")),
            "{:?}",
            synced.logs
        );
        for origin in [&widgets, &lib] {
            assert!(!has_branch(origin, &p.home, "fctl/1-test").await?);
        }
        assert_eq!((synced.state.ahead, synced.state.pushed), (1, false));
        // lib's work stays where the agent left it, and lib reads as read only.
        let repos = synced.state.repos.context("repos")?;
        assert_eq!(repos[0].access, RepoAccess::Read);
        assert_eq!(repos[0].branch.as_deref(), Some("main"));
        Ok(())
    }

    #[tokio::test]
    async fn an_agent_held_to_reading_drops_the_token_that_could_write() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let p = sandbox(dir.path())?;
        configure_git(&p, &spec()?.git_author).await?;
        write_routes(&p, &spec()?)?;
        write_github_tokens(&p, Some(&cred("ghs_write")), Some(&cred("ghs_read")))?;
        // Its project now holds its agents to reading: one read-only token.
        write_github_tokens(&p, None, Some(&cred("ghs_read2")))?;
        assert_eq!(
            token_for(&p.home, Some("acme/widgets.git")).await?,
            Some("ghs_read2".into())
        );
        let hosts = std::fs::read_to_string(p.home.join(".config/gh/hosts.yml"))?;
        assert!(hosts.contains("oauth_token: ghs_read2"));
        Ok(())
    }

    #[tokio::test]
    async fn reading_the_state_leaves_the_index_to_the_agent() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let p = sandbox(dir.path())?;
        let spec = spec()?;
        configure_git(&p, &spec.git_author).await?;
        let origin = fake_github(dir.path(), &p.home, "acme/widgets").await?;
        std::fs::write(origin.join("a.txt"), "one\n")?;
        git_ok_in(&origin, &p.home, &["add", "a.txt"]).await?;
        git_ok_in(&origin, &p.home, &["commit", "-q", "-m", "Add a"]).await?;
        let (log, _logs) = mpsc::unbounded_channel();
        prepare(&p, &spec, &log).await?;
        std::fs::write(p.workdir.join("b.txt"), "two\n")?;
        git_ok(&p, &["add", "b.txt"]).await?;
        git_ok(&p, &["commit", "-q", "-m", "Add b"]).await?;
        // a.txt keeps its content with newer stat data, as after a
        // formatter ran over it; b.txt changes.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(p.workdir.join("a.txt"))?
            .set_modified(later)?;
        std::fs::write(p.workdir.join("b.txt"), "two\nthree\n")?;
        let index = p.workdir.join(".git/index");
        let before = std::fs::read(&index)?;

        let st = state(&p, &spec, &Pulls::default()).await?;
        assert!(
            std::fs::read(&index)? == before,
            "reading the state rewrote the index"
        );
        let files: Vec<(&str, &str, Option<u32>)> = st
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.status.as_str(), f.additions))
            .collect();
        assert_eq!(files, [("b.txt", "added", Some(2))]);
        assert_eq!((st.ahead, st.uncommitted), (1, 1));
        // `git diff` on the index itself rewrites it, under its lock.
        git_ok(&p, &["diff", "--numstat", "origin/main"]).await?;
        assert!(std::fs::read(&index)? != before);
        Ok(())
    }

    #[tokio::test]
    async fn a_refresh_while_claude_works_only_reads() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let p = sandbox(dir.path())?;
        let spec = spec_with(serde_json::json!([
            { "owner": "acme", "name": "lib", "access": "write" }
        ]))?;
        configure_git(&p, &spec.git_author).await?;
        let widgets = fake_github(dir.path(), &p.home, "acme/widgets").await?;
        let lib = fake_github(dir.path(), &p.home, "acme/lib").await?;
        let (log, _logs) = mpsc::unbounded_channel();
        prepare(&p, &spec, &log).await?;
        let clone = p.repos.join("acme/lib");
        let to = clone.display().to_string();
        let url = "https://github.com/acme/lib.git";
        assert!(
            run(&p.home, &p.home, "git", &["clone", "-q", url, &to])
                .await?
                .ok
        );
        // The agent commits in both, in the clone on its default branch.
        for dir in [&p.workdir, &clone] {
            std::fs::write(dir.join("change.txt"), "hello\n")?;
            git_ok_in(dir, &p.home, &["add", "change.txt"]).await?;
            git_ok_in(dir, &p.home, &["commit", "-q", "-m", "Change"]).await?;
        }

        // Even after `gh pr create`, nothing is pushed or looked up.
        let working = Moment::Working { opened_pr: true };
        let synced = sync(&p, &spec, true, working, None, &mut Pulls::default()).await?;
        assert!(synced.logs.is_empty(), "{:?}", synced.logs);
        let st = synced.state;
        assert_eq!((st.ahead, st.pushed), (1, false));
        let repos = st.repos.context("repos")?;
        assert_eq!(repos[0].branch.as_deref(), Some("main"));
        assert_eq!((repos[0].ahead, repos[0].pushed), (1, false));
        let session_branch = ["rev-parse", "--verify", "--quiet", "refs/heads/fctl/1-test"];
        for dir in [&widgets, &lib, &clone] {
            assert!(!run(dir, &p.home, "git", &session_branch).await?.ok);
        }
        Ok(())
    }

    #[test]
    fn looks_up_pull_requests_only_where_one_can_have_appeared() {
        let branch = Some("fctl/1-test".to_owned());
        let (first, second) = (Some("a1".to_owned()), Some("b2".to_owned()));
        let pushed = on_remote(&branch, &first, true);
        let moved = on_remote(&branch, &second, true);
        let committed = on_remote(&branch, &second, false);
        let working = Moment::Working { opened_pr: false };
        // A branch pushed since the last report, or whose pushed head moved.
        assert!(working.looks_up(pushed, Some(None)));
        assert!(working.looks_up(moved, Some(pushed)));
        // A repository the last report didn't have.
        assert!(working.looks_up(pushed, None));
        // Nothing new for GitHub.
        assert!(!working.looks_up(pushed, Some(pushed)));
        assert!(!working.looks_up(committed, Some(pushed)));
        // After `gh pr create`, every pushed branch.
        let opened = Moment::Working { opened_pr: true };
        assert!(opened.looks_up(pushed, Some(pushed)));
        assert!(!opened.looks_up(committed, Some(pushed)));
        // When a turn ends, every pushed branch; none when the session stops.
        assert!(Moment::TurnEnded.looks_up(pushed, Some(pushed)));
        assert!(!Moment::TurnEnded.looks_up(committed, None));
        assert!(!Moment::Stopping.looks_up(moved, Some(None)));
    }

    #[test]
    fn looks_up_a_deleted_branch_once_after_a_start_and_after_each_turn() {
        let branch = Some("fctl/1-test".to_owned());
        let head = Some("a1".to_owned());
        let working = Moment::Working { opened_pr: false };
        // The first report after fctl-agent started, which didn't see it go.
        assert!(working.looks_up_deleted(None));
        assert!(!working.looks_up_deleted(Some(None)));
        assert!(!working.looks_up_deleted(Some(on_remote(&branch, &head, true))));
        assert!(Moment::Working { opened_pr: true }.looks_up_deleted(Some(None)));
        assert!(Moment::TurnEnded.looks_up_deleted(Some(None)));
        assert!(!Moment::Stopping.looks_up_deleted(None));
    }

    #[tokio::test]
    async fn a_merged_branch_shows_what_it_merged_and_is_not_pushed_again() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut p = sandbox(dir.path())?;
        let gh = FakeGh::install(&mut p, dir.path())?;
        let spec = spec()?;
        configure_git(&p, &spec.git_author).await?;
        let origin = fake_github(dir.path(), &p.home, "acme/widgets").await?;
        let (log, _logs) = mpsc::unbounded_channel();
        prepare(&p, &spec, &log).await?;
        let main = git_ok_in(&origin, &p.home, &["rev-parse", "main"]).await?;
        let first = commit(&p.workdir, &p.home, "a.txt", "one\ntwo\n", "Add a.txt").await?;
        gh.knows(1, "fctl/1-test", "OPEN", &first, main.trim())?;
        let mut pulls = Pulls::default();
        let ended = Moment::TurnEnded;
        let synced = sync(&p, &spec, true, ended, None, &mut pulls).await?;
        let st = synced.state;
        assert_eq!(log_texts(&synced.logs), ["Pushed fctl/1-test"]);
        assert_eq!(changes(&st), ["added a.txt +2 -0"]);
        assert_eq!((st.ahead, st.pushed, st.unmerged), (1, true, None));
        assert_eq!(
            st.pull_request.and_then(|p| p.state).as_deref(),
            Some("open")
        );

        // GitHub squashes it onto main and deletes the branch. The session
        // resumes on its checkout, with what fctl-agent kept, and fetches.
        let before = merge_on_github(&origin, &p.home, "fctl/1-test", true).await?;
        gh.knows(1, "fctl/1-test", "MERGED", &first, &before)?;
        prepare(&p, &spec, &log).await?;
        let working = Moment::Working { opened_pr: false };
        let st = sync(&p, &spec, true, working, None, &mut pulls)
            .await?
            .state;
        // The work it merged, which main has now.
        assert_eq!(changes(&st), ["added a.txt +2 -0"]);
        assert_eq!((st.ahead, st.behind, st.pushed), (1, 1, false));
        let state = st.pull_request.as_ref().and_then(|p| p.state.as_deref());
        assert_eq!(state, Some("merged"));
        assert_eq!(st.unmerged, Some(0));
        // A turn ends: nothing new to push, and the branch stays deleted.
        let synced = sync(&p, &spec, true, ended, Some(&st), &mut pulls).await?;
        assert!(synced.logs.is_empty(), "{:?}", synced.logs);
        assert!(!has_branch(&origin, &p.home, "fctl/1-test").await?);

        // The agent goes on on the merged branch: its new commit stays here,
        // and the transcript says why.
        commit(&p.workdir, &p.home, "b.txt", "three\n", "Add b.txt").await?;
        let synced = sync(&p, &spec, true, ended, Some(&synced.state), &mut pulls).await?;
        let st = synced.state;
        assert_eq!(
            log_texts(&synced.logs),
            ["Didn't push fctl/1-test: its pull request #1 is merged"]
        );
        assert_eq!(changes(&st), ["added a.txt +2 -0", "added b.txt +1 -0"]);
        assert_eq!((st.ahead, st.pushed, st.unmerged), (2, false, Some(1)));
        assert!(!has_branch(&origin, &p.home, "fctl/1-test").await?);
        // It says so once.
        let again = sync(&p, &spec, true, ended, Some(&st), &mut pulls).await?;
        assert!(again.logs.is_empty(), "{:?}", again.logs);
        // Merged is final: no more lookups.
        let lookups = gh.calls().lines().count();
        sync(&p, &spec, true, ended, Some(&again.state), &mut pulls).await?;
        assert_eq!(gh.calls().lines().count(), lookups);

        // An agent that kept nothing, as after an upgrade, tells from the
        // deleted branch to ask GitHub.
        let mut fresh = Pulls::default();
        let st = sync(&p, &spec, true, working, None, &mut fresh)
            .await?
            .state;
        assert_eq!(
            st.pull_request.and_then(|p| p.state).as_deref(),
            Some("merged")
        );
        let synced = sync(&p, &spec, true, ended, None, &mut fresh).await?;
        assert!(!has_branch(&origin, &p.home, "fctl/1-test").await?);
        assert_eq!(synced.state.unmerged, Some(1));
        Ok(())
    }

    #[tokio::test]
    async fn a_branch_pushed_without_an_upstream_is_not_pushed_again_once_merged() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut p = sandbox(dir.path())?;
        let gh = FakeGh::install(&mut p, dir.path())?;
        let spec = spec()?;
        configure_git(&p, &spec.git_author).await?;
        let origin = fake_github(dir.path(), &p.home, "acme/widgets").await?;
        let (log, _logs) = mpsc::unbounded_channel();
        prepare(&p, &spec, &log).await?;
        let main = git_ok_in(&origin, &p.home, &["rev-parse", "main"]).await?;
        let first = commit(&p.workdir, &p.home, "a.txt", "one\n", "Add a.txt").await?;
        // The agent pushes it itself, naming the branch, which sets no
        // upstream, and opens its pull request.
        git_ok(&p, &["push", "-q", "origin", "fctl/1-test"]).await?;
        gh.knows(1, "fctl/1-test", "OPEN", &first, main.trim())?;
        let mut pulls = Pulls::default();
        let ended = Moment::TurnEnded;
        let st = sync(&p, &spec, true, ended, None, &mut pulls).await?.state;
        assert_eq!(st.pull_request.as_ref().map(|p| p.number), Some(1));
        // It merges; a fetch prunes the branch, and the agent commits more.
        let before = merge_on_github(&origin, &p.home, "fctl/1-test", true).await?;
        gh.knows(1, "fctl/1-test", "MERGED", &first, &before)?;
        git_ok(&p, &["fetch", "-q", "--prune", "origin"]).await?;
        commit(&p.workdir, &p.home, "b.txt", "two\n", "Add b.txt").await?;
        // Before pushing, fctl-agent asks GitHub about the pull request it
        // saw open.
        let synced = sync(&p, &spec, true, ended, Some(&st), &mut pulls).await?;
        assert_eq!(
            log_texts(&synced.logs),
            ["Didn't push fctl/1-test: its pull request #1 is merged"]
        );
        assert!(!has_branch(&origin, &p.home, "fctl/1-test").await?);
        assert_eq!(synced.state.unmerged, Some(1));
        Ok(())
    }

    #[tokio::test]
    async fn a_branch_merged_with_a_merge_commit_shows_what_it_merged() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut p = sandbox(dir.path())?;
        let gh = FakeGh::install(&mut p, dir.path())?;
        let spec = spec()?;
        configure_git(&p, &spec.git_author).await?;
        let origin = fake_github(dir.path(), &p.home, "acme/widgets").await?;
        let (log, _logs) = mpsc::unbounded_channel();
        prepare(&p, &spec, &log).await?;
        let main = git_ok_in(&origin, &p.home, &["rev-parse", "main"]).await?;
        let first = commit(&p.workdir, &p.home, "a.txt", "one\ntwo\n", "Add a.txt").await?;
        gh.knows(1, "fctl/1-test", "OPEN", &first, main.trim())?;
        let mut pulls = Pulls::default();
        sync(&p, &spec, true, Moment::TurnEnded, None, &mut pulls).await?;
        let before = merge_on_github(&origin, &p.home, "fctl/1-test", false).await?;
        gh.knows(1, "fctl/1-test", "MERGED", &first, &before)?;
        prepare(&p, &spec, &log).await?;
        // Main has the branch's commit now, so from its merge base with the
        // branch nothing changed. From main before the merge, what merged.
        assert!(state(&p, &spec, &Pulls::default()).await?.files.is_empty());
        let working = Moment::Working { opened_pr: false };
        let st = sync(&p, &spec, true, working, None, &mut pulls)
            .await?
            .state;
        assert_eq!(changes(&st), ["added a.txt +2 -0"]);
        assert_eq!((st.ahead, st.unmerged), (0, Some(0)));
        assert_eq!(
            st.pull_request.and_then(|p| p.state).as_deref(),
            Some("merged")
        );
        Ok(())
    }

    #[tokio::test]
    async fn changes_are_the_branch_s_own_when_the_base_moves_on() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let p = sandbox(dir.path())?;
        let spec = spec()?;
        configure_git(&p, &spec.git_author).await?;
        let origin = fake_github(dir.path(), &p.home, "acme/widgets").await?;
        let (log, _logs) = mpsc::unbounded_channel();
        prepare(&p, &spec, &log).await?;
        commit(&p.workdir, &p.home, "a.txt", "one\ntwo\n", "Add a.txt").await?;
        // Someone else's change lands on main, and the clone fetches it.
        commit(&origin, &p.home, "theirs.txt", "1\n2\n3\n", "Their change").await?;
        git_ok(&p, &["fetch", "-q", "origin"]).await?;
        // Uncommitted and untracked work counts as it is now.
        std::fs::write(p.workdir.join("a.txt"), "one\ntwo\nthree\n")?;
        std::fs::write(p.workdir.join("new.txt"), "x\n")?;
        let st = state(&p, &spec, &Pulls::default()).await?;
        assert_eq!(
            changes(&st),
            ["added a.txt +3 -0", "untracked new.txt +1 -0"]
        );
        assert_eq!((st.ahead, st.behind, st.uncommitted), (1, 1, 2));
        Ok(())
    }

    #[tokio::test]
    async fn pull_requests_from_other_branches_are_followed() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut p = sandbox(dir.path())?;
        let gh = FakeGh::install(&mut p, dir.path())?;
        let spec = spec()?;
        configure_git(&p, &spec.git_author).await?;
        let origin = fake_github(dir.path(), &p.home, "acme/widgets").await?;
        let (log, _logs) = mpsc::unbounded_channel();
        prepare(&p, &spec, &log).await?;
        let main = git_ok_in(&origin, &p.home, &["rev-parse", "main"]).await?;
        let first = commit(&p.workdir, &p.home, "a.txt", "a\n", "First").await?;
        gh.knows(1, "fctl/1-test", "OPEN", &first, main.trim())?;
        let mut pulls = Pulls::default();
        let ended = Moment::TurnEnded;
        let st = sync(&p, &spec, true, ended, None, &mut pulls).await?.state;
        // A second branch in a worktree beside the checkout, pushed, with
        // its own pull request.
        let wt = dir.path().join("second");
        let wt_arg = wt.display().to_string();
        let add = [
            "worktree",
            "add",
            "-q",
            "-b",
            "fctl/1-second",
            &wt_arg,
            "origin/main",
        ];
        git_ok(&p, &add).await?;
        let second = commit(&wt, &p.home, "b.txt", "b\n", "Second").await?;
        git_ok_in(
            &wt,
            &p.home,
            &["push", "-q", "-u", "origin", "fctl/1-second"],
        )
        .await?;
        gh.knows(2, "fctl/1-second", "OPEN", &second, main.trim())?;
        // `gh pr create --head fctl/1-second` printed its URL: the refresh
        // after it looks that up.
        let printed = fctl_domain::pulls::named_in("https://github.com/acme/widgets/pull/2\n");
        assert!(pulls.follow(printed, &spec));
        let working = Moment::Working { opened_pr: true };
        let st = sync(&p, &spec, true, working, Some(&st), &mut pulls)
            .await?
            .state;
        let found = |st: &GitState| -> Vec<(u64, Option<String>)> {
            fctl_domain::pulls::found(st)
                .into_iter()
                .map(|p| (p.number, p.state))
                .collect()
        };
        assert_eq!(
            found(&st),
            [(1, Some("open".into())), (2, Some("open".into()))]
        );
        assert!(
            gh.calls()
                .contains("pr view https://github.com/acme/widgets/pull/2 --json")
        );
        // Changes are the checked-out branch's.
        assert_eq!(changes(&st), ["added a.txt +1 -0"]);
        // The second merges: its state follows when a turn ends, and then
        // the lookups stop.
        gh.knows(2, "fctl/1-second", "MERGED", &second, main.trim())?;
        let st = sync(&p, &spec, true, ended, Some(&st), &mut pulls)
            .await?
            .state;
        assert_eq!(
            found(&st),
            [(1, Some("open".into())), (2, Some("merged".into()))]
        );
        let lookups = gh.calls().matches("pull/2").count();
        sync(&p, &spec, true, ended, Some(&st), &mut pulls).await?;
        assert_eq!(gh.calls().matches("pull/2").count(), lookups);
        Ok(())
    }

    #[tokio::test]
    async fn an_older_gh_still_finds_pull_requests() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut p = sandbox(dir.path())?;
        let gh = FakeGh::install(&mut p, dir.path())?;
        let spec = spec()?;
        configure_git(&p, &spec.git_author).await?;
        let origin = fake_github(dir.path(), &p.home, "acme/widgets").await?;
        let (log, _logs) = mpsc::unbounded_channel();
        prepare(&p, &spec, &log).await?;
        let main = git_ok_in(&origin, &p.home, &["rev-parse", "main"]).await?;
        let first = commit(&p.workdir, &p.home, "a.txt", "a\n", "First").await?;
        gh.knows(1, "fctl/1-test", "OPEN", &first, main.trim())?;
        gh.grow_old()?;
        let mut pulls = Pulls::default();
        let st = sync(&p, &spec, true, Moment::TurnEnded, None, &mut pulls)
            .await?
            .state;
        assert_eq!(
            st.pull_request.and_then(|p| p.state).as_deref(),
            Some("open")
        );
        let calls = gh.calls();
        assert!(
            calls.contains("headRefName,headRefOid,baseRefOid"),
            "{calls}"
        );
        assert!(
            calls.contains("--json number,url,title,state,isDraft\n"),
            "{calls}"
        );
        Ok(())
    }
}
