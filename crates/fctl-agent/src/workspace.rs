//! The session's repository: cloning, branches, the setup script, git state
//! after each turn, pushing, and the credentials git and `gh` use.

use anyhow::{Context, Result, bail};
use fctl_proto::event::{CommitInfo, FileChange, GitState, LogLevel};
use fctl_proto::runner::{GitIdentity, GithubCredential, SessionSpec};
use fctl_proto::session::PullRequestRef;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

/// Paths fctl-agent manages inside the sandbox.
#[derive(Clone)]
pub struct Paths {
    pub workdir: PathBuf,
    pub home: PathBuf,
    /// Holds the GitHub token the credential helper reads.
    pub token_file: PathBuf,
    pub helper: PathBuf,
}

impl Paths {
    pub fn new(workdir: &Path, home: &Path, state: &Path) -> Self {
        Self {
            workdir: workdir.to_path_buf(),
            home: home.to_path_buf(),
            token_file: state.join("github-token"),
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
    let o = git(p, args).await?;
    if !o.ok {
        bail!("git {} failed: {}", args.join(" "), o.stderr.trim());
    }
    Ok(o.stdout)
}

/// Writes the GitHub token where git's credential helper and `gh` read it.
pub fn write_github_token(p: &Paths, cred: &GithubCredential) -> Result<()> {
    if let Some(dir) = p.token_file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_private(&p.token_file, &cred.token)?;
    let gh_dir = p.home.join(".config/gh");
    std::fs::create_dir_all(&gh_dir)?;
    let hosts = format!(
        "github.com:\n    oauth_token: {}\n    git_protocol: https\n    user: x-access-token\n",
        cred.token
    );
    write_private(&gh_dir.join("hosts.yml"), &hosts)?;
    Ok(())
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
    let helper = format!(
        "#!/bin/sh\n# Written by fctl-agent: hands git the session's GitHub token.\nif [ \"$1\" = get ]; then\n  echo username=x-access-token\n  echo \"password=$(cat '{}')\"\nfi\n",
        p.token_file.display()
    );
    if let Some(dir) = p.helper.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&p.helper, helper)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p.helper, std::fs::Permissions::from_mode(0o700))?;
    }
    let helper_path = p.helper.display().to_string();
    for (k, v) in [
        ("user.name", author.name.as_str()),
        ("user.email", author.email.as_str()),
        ("credential.https://github.com.helper", ""),
        ("init.defaultBranch", "main"),
        ("push.autoSetupRemote", "true"),
        ("advice.detachedHead", "false"),
        ("core.quotePath", "false"),
    ] {
        run(&p.home, &p.home, "git", &["config", "--global", k, v]).await?;
    }
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

/// Clones (or reuses) the repository and checks out the session branch.
pub async fn prepare(p: &Paths, spec: &SessionSpec, log: &LogSink) -> Result<()> {
    std::fs::create_dir_all(&p.workdir)?;
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
        git_ok(
            p,
            &[
                "checkout",
                "-q",
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

/// The repository's state relative to the base branch.
pub async fn state(p: &Paths, spec: &SessionSpec) -> Result<GitState> {
    if !p.workdir.join(".git").exists() {
        return Ok(GitState::default());
    }
    let branch = git_ok(p, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .ok()
        .map(|s| s.trim().to_owned());
    let head = git_ok(p, &["rev-parse", "HEAD"])
        .await
        .ok()
        .map(|s| s.trim().to_owned());
    let base_ref = format!("origin/{}", spec.base_branch);
    let has_base = git(p, &["rev-parse", "--verify", "--quiet", &base_ref])
        .await?
        .ok;
    let (behind, ahead) = if has_base && head.is_some() {
        let counts = git_ok(
            p,
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
            git_ok(p, &["rev-list", "--count", "HEAD"])
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
        let numstat = git(p, &["diff", "--numstat", "-M", &diff_target]).await?;
        let names = git(p, &["diff", "--name-status", "-M", &diff_target]).await?;
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
    let porcelain = git(p, &["status", "--porcelain"]).await?;
    let mut uncommitted = 0u32;
    for line in porcelain.stdout.lines() {
        uncommitted += 1;
        if let Some(path) = line.strip_prefix("?? ") {
            let path = path.trim_matches('"').to_owned();
            if !path.ends_with('/') && !files.iter().any(|f| f.path == path) {
                let lines = std::fs::read_to_string(p.workdir.join(&path))
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
    let last_commit = git_ok(p, &["log", "-1", "--format=%H%x00%s"])
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
        Some(b) => git_ok(
            p,
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
        base: Some(spec.base_branch.clone()),
        pushed: head.is_some() && remote_head == head,
        head,
        ahead,
        behind,
        files,
        uncommitted,
        last_commit,
        pull_request: None,
    })
}

/// Pushes the session branch when it has commits the remote lacks.
pub async fn push(p: &Paths, st: &GitState) -> Result<bool> {
    let Some(branch) = st.branch.as_deref() else {
        return Ok(false);
    };
    if st.pushed || st.head.is_none() || branch == "HEAD" {
        return Ok(false);
    }
    let o = git(p, &["push", "--quiet", "-u", "origin", branch]).await?;
    if !o.ok {
        bail!("git push failed: {}", o.stderr.trim());
    }
    Ok(true)
}

/// The pull request of the session branch, through `gh`.
pub async fn pull_request(p: &Paths, branch: &str) -> Option<PullRequestRef> {
    let o = run(
        &p.workdir,
        &p.home,
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
