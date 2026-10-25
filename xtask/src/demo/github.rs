//! A stand-in GitHub for the demo's hosts: the cast's repositories as bare
//! ones on disk, which git reaches in place of `https://github.com/` (an
//! `insteadOf` in the hosts' environment, which their agents and those
//! agents' git inherit), and a `gh` first on their PATH that answers
//! `gh pr view` with the pull requests the cast's sessions open.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Github {
    /// The bare repositories, as `<owner>/<name>.git`, and `pulls/`.
    pub dir: PathBuf,
    /// Where the stand-in `gh` is.
    pub bin: PathBuf,
}

/// Answers `gh pr view <branch or URL> --json …` from `pulls/`, by the
/// branch or URL it is asked about; anything else fails, as gh does
/// without a GitHub to ask.
const GH: &str = r#"#!/bin/sh
# The demo's stand-in for gh (cargo xtask demo).
here=$(dirname "$0")
pulls="$here/../github/pulls"
[ "$1 $2" = "pr view" ] || { echo "gh: only 'gh pr view' answers in the demo" >&2; exit 1; }
answer="$pulls/$(printf '%s' "$3" | tr -c 'A-Za-z0-9._-' '_')"
[ -f "$answer" ] || { echo "no pull requests found for branch \"$3\"" >&2; exit 1; }
cat "$answer"
"#;

impl Github {
    pub fn new(state: &Path) -> Self {
        Self {
            dir: state.join("github"),
            bin: state.join("bin"),
        }
    }

    /// Makes the stand-in: its `gh`, and an empty place for repositories.
    pub fn install(&self) -> Result<()> {
        std::fs::create_dir_all(self.dir.join("pulls"))?;
        std::fs::create_dir_all(&self.bin)?;
        let gh = self.bin.join("gh");
        std::fs::write(&gh, GH)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    }

    /// What makes git reach the stand-in in place of GitHub, for the hosts'
    /// environment and the commands replays run.
    pub fn git_env(&self) -> Vec<(String, String)> {
        vec![
            ("GIT_CONFIG_COUNT".into(), "1".into()),
            (
                "GIT_CONFIG_KEY_0".into(),
                format!("url.{}/.insteadOf", self.dir.display()),
            ),
            ("GIT_CONFIG_VALUE_0".into(), "https://github.com/".into()),
        ]
    }

    /// Creates `owner/name` with the files in `files` (when it exists) in
    /// its first commit on `main`, which `author` made at `at` (seconds
    /// since the epoch).
    pub fn create(
        &self,
        repo: &str,
        files: Option<&Path>,
        author: (&str, &str),
        at: i64,
    ) -> Result<()> {
        let bare = self.dir.join(format!("{repo}.git"));
        if bare.exists() {
            return Ok(());
        }
        let work = tempdir(&self.dir.join(".work"), repo)?;
        match files {
            Some(src) if src.is_dir() => copy_dir(src, &work)?,
            _ => {
                let name = repo.rsplit('/').next().unwrap_or(repo);
                std::fs::write(work.join("README.md"), format!("# {name}\n"))?;
            }
        }
        let date = format!("@{at} +0000");
        let git = |args: &[&str], dir: &Path| -> Result<()> {
            let out = Command::new("git")
                .args(args)
                .current_dir(dir)
                .env("GIT_AUTHOR_NAME", author.0)
                .env("GIT_AUTHOR_EMAIL", author.1)
                .env("GIT_COMMITTER_NAME", author.0)
                .env("GIT_COMMITTER_EMAIL", author.1)
                .env("GIT_AUTHOR_DATE", &date)
                .env("GIT_COMMITTER_DATE", &date)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()
                .context("running git")?;
            if !out.status.success() {
                bail!(
                    "git {} in {}: {}",
                    args.join(" "),
                    dir.display(),
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            Ok(())
        };
        std::fs::create_dir_all(bare.parent().unwrap_or(&self.dir))?;
        git(
            &[
                "init",
                "-q",
                "--bare",
                "-b",
                "main",
                &bare.display().to_string(),
            ],
            &self.dir,
        )?;
        git(&["init", "-q", "-b", "main"], &work)?;
        git(&["add", "-A"], &work)?;
        git(&["commit", "-q", "-m", "Initial commit"], &work)?;
        git(&["push", "-q", &bare.display().to_string(), "main"], &work)?;
        std::fs::remove_dir_all(&work).ok();
        Ok(())
    }

    /// Tells `gh pr view` about a pull request, by its branch and its URL.
    pub fn pull(&self, repo: &str, number: u64, title: &str, branch: &str) -> Result<()> {
        let url = format!("https://github.com/{repo}/pull/{number}");
        let now = crate::demo::iso(crate::demo::now_ms());
        let answer: Value = json!({
            "number": number,
            "url": url,
            "title": title,
            "state": "OPEN",
            "isDraft": false,
            "createdAt": now,
            "updatedAt": now,
            "headRefName": branch,
            "baseRefName": "main",
        });
        for selector in [branch, url.as_str()] {
            std::fs::write(
                self.dir.join("pulls").join(sanitized(selector)),
                answer.to_string(),
            )?;
        }
        Ok(())
    }

    /// The commits on `branch` of `repo` that `main` doesn't have, oldest
    /// first: the full SHA of each.
    pub fn commits(&self, repo: &str, branch: &str) -> Result<Vec<String>> {
        let bare = self.dir.join(format!("{repo}.git"));
        let out = Command::new("git")
            .args(["rev-list", "--reverse", &format!("main..{branch}")])
            .current_dir(&bare)
            .output()
            .context("running git rev-list")?;
        if !out.status.success() {
            return Ok(Vec::new());
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_owned)
            .collect())
    }
}

/// A selector as `gh`'s stand-in names its file.
fn sanitized(selector: &str) -> String {
    selector
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn tempdir(parent: &Path, repo: &str) -> Result<PathBuf> {
    let dir = parent.join(sanitized(repo));
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    for e in std::fs::read_dir(src)
        .with_context(|| format!("reading {}", src.display()))?
        .flatten()
    {
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            std::fs::create_dir_all(&to)?;
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::sanitized;

    #[test]
    fn selectors_name_files_as_the_stand_in_reads_them() {
        assert_eq!(
            sanitized("https://github.com/romanescohq/shop/pull/1291"),
            "https___github.com_romanescohq_shop_pull_1291"
        );
        assert_eq!(
            sanitized("fctl/231-add-danish-to-the-shop"),
            "fctl_231-add-danish-to-the-shop"
        );
    }
}
