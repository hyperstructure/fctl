//! Session snapshots in the sandbox (docs/architecture.md#session-snapshots):
//! what fctl-agent saves as the sandbox stops, so that the session resumes
//! on another runner as it was, and how it puts that back in a sandbox
//! that doesn't have it.
//!
//! A snapshot is a gzipped tar, sealed with the session's key
//! (`fctl_domain::snapshots`):
//!
//! - `meta.json`: the session, the Claude Code that wrote the
//!   conversation, where it was, and each repository's work;
//! - `claude/projects/<dir>/<session>.jsonl` and `claude/projects/<dir>/<session>/…`:
//!   Claude Code's conversation and what it keeps beside it (subagents'
//!   conversations, long tool results), and `claude/todos/<session>…`;
//! - `repos/<n>.bundle`: each repository's work that isn't on GitHub, as a
//!   git bundle of a commit of its working tree (`refs/fctl/wip/<session>`)
//!   on top of its commits not pushed. Git keeps what a tarball wouldn't:
//!   deletions, modes, binary files, which files were untracked.
//!
//! Restoring fetches each bundle into the fresh clone and, where the
//! branch is where the snapshot left it or behind it, checks the work out
//! and resets to the commit before it, so that uncommitted work is
//! uncommitted again. Where the branch moved on elsewhere, the work stays
//! at `refs/fctl/wip/<session>`, and Claude hears where.

use crate::workspace::Paths;
use anyhow::{Context, Result, anyhow, bail};
use fctl_domain::snapshots::{self as snap, HEADER, Opener, SEGMENT, Sealer, TAG};
use fctl_proto::agent::SnapshotSaved;
use fctl_proto::runner::SessionSpec;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// What `meta.json` says.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub version: u32,
    pub claude_session_id: String,
    pub claude_version: Option<String>,
    /// Where the conversation was: Claude Code's directory for the working
    /// directory, under `~/.claude/projects`.
    pub conversation_dir: Option<String>,
    pub workdir: String,
    pub repos: Vec<RepoMeta>,
    /// Images it left out of the conversation, to stay within the limit.
    #[serde(default)]
    pub images_left_out: usize,
}

/// A repository's work in a snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoMeta {
    /// Where it is: `""` for the working directory, or its path under the
    /// home directory (`repos/acme/api`).
    pub dir: String,
    /// The commit its branch was at.
    pub head: String,
    /// The commit of its working tree, on top of `head`.
    pub wip: String,
    /// Its bundle in the snapshot.
    pub bundle: String,
}

/// The ref a session's work in progress is kept at.
pub fn wip_ref(session: &str) -> String {
    format!("refs/fctl/wip/{session}")
}

/// Claude Code's directory for conversations in `workdir`: the path with
/// every character but letters and digits made a `-`
/// (`/home/agent/workspace` is `-home-agent-workspace`).
pub fn conversation_dir(workdir: &Path) -> String {
    workdir
        .display()
        .to_string()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Where Claude Code keeps conversation `id` under `home`, if it does: its
/// project directory's name.
fn find_conversation(home: &Path, id: &str) -> Option<String> {
    let file = format!("{id}.jsonl");
    std::fs::read_dir(home.join(".claude/projects"))
        .ok()?
        .flatten()
        .find(|d| d.path().join(&file).is_file())
        .map(|d| d.file_name().to_string_lossy().into_owned())
}

/// The repositories a session works in that are here: the working
/// directory first, then those under `~/repos`, as `(dir, path)`.
fn repos(paths: &Paths, spec: &SessionSpec) -> Vec<(String, PathBuf)> {
    let mut out = vec![(String::new(), paths.workdir.clone())];
    for r in spec.project.repos.iter().flatten() {
        // An owner-wide grant names no repository to have cloned.
        let Some(name) = &r.name else { continue };
        let path = paths.repos.join(&r.owner).join(name);
        if path.join(".git").exists() {
            let dir = path
                .strip_prefix(&paths.home)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| path.display().to_string());
            out.push((dir, path));
        }
    }
    out.into_iter()
        .filter(|(_, p)| p.join(".git").exists())
        .collect()
}

/// Runs git in `dir` as the session's user, with `env`: its stdout as
/// bytes, or why it failed.
async fn git(home: &Path, dir: &Path, args: &[&str], env: &[(&str, &Path)]) -> Result<Vec<u8>> {
    let out = crate::user::command("git")
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("GIT_TERMINAL_PROMPT", "0")
        // The work-in-progress commit stays in the sandbox and the
        // snapshot: it needs an author, not the session's.
        .env("GIT_AUTHOR_NAME", "fctl")
        .env("GIT_AUTHOR_EMAIL", "fctl@localhost")
        .env("GIT_COMMITTER_NAME", "fctl")
        .env("GIT_COMMITTER_EMAIL", "fctl@localhost")
        .envs(env.iter().map(|(k, v)| (*k, v.as_os_str())))
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .context("running git")?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

async fn git_text(home: &Path, dir: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&git(home, dir, args, &[]).await?)
        .trim()
        .to_owned())
}

/// A repository's work that isn't on GitHub, as a bundle: a commit of its
/// working tree, untracked files with it but what it ignores, on top of
/// its commits not pushed. `None` when it has neither.
async fn work_of(home: &Path, repo: &Path, session: &str) -> Result<Option<(RepoMeta, Vec<u8>)>> {
    let head = git_text(home, repo, &["rev-parse", "--verify", "HEAD"]).await?;
    let changed = !git(home, repo, &["status", "--porcelain"], &[])
        .await?
        .is_empty();
    let unpushed = git_text(
        home,
        repo,
        &["rev-list", "--count", "HEAD", "--not", "--remotes"],
    )
    .await?
    .parse::<u64>()
    .unwrap_or(1)
        > 0;
    if !changed && !unpushed {
        return Ok(None);
    }
    // An index of its own, so that the repository's stays as it is.
    let index = repo.join(".git/fctl-snapshot-index");
    let _ = std::fs::remove_file(&index);
    let env = [("GIT_INDEX_FILE", index.as_path())];
    let made = async {
        git(home, repo, &["read-tree", "HEAD"], &env).await?;
        git(home, repo, &["add", "--all"], &env).await?;
        let tree = String::from_utf8_lossy(&git(home, repo, &["write-tree"], &env).await?)
            .trim()
            .to_owned();
        let message = format!("fctl: work in progress of session {session}");
        let wip = String::from_utf8_lossy(
            &git(
                home,
                repo,
                &["commit-tree", &tree, "-p", &head, "-m", &message],
                &env,
            )
            .await?,
        )
        .trim()
        .to_owned();
        anyhow::Ok(wip)
    }
    .await;
    let _ = std::fs::remove_file(&index);
    let wip = made?;
    let r = wip_ref(session);
    git(home, repo, &["update-ref", &r, &wip], &[]).await?;
    let bundle = git(
        home,
        repo,
        &["bundle", "create", "-", &r, "--not", "--remotes"],
        &[],
    )
    .await?;
    Ok(Some((
        RepoMeta {
            dir: String::new(),
            head,
            wip,
            bundle: String::new(),
        },
        bundle,
    )))
}

/// What a snapshot holds before it is packed.
struct Parts {
    meta: Meta,
    /// The conversation's files, by their path in the snapshot.
    files: Vec<(String, Vec<u8>)>,
    bundles: Vec<(String, Vec<u8>)>,
}

/// Collects what a snapshot of the session holds.
async fn collect(paths: &Paths, spec: &SessionSpec, claude_version: Option<&str>) -> Result<Parts> {
    let id = &spec.claude_session_id;
    let mut meta = Meta {
        version: 1,
        claude_session_id: id.clone(),
        claude_version: claude_version.map(str::to_owned),
        workdir: paths.workdir.display().to_string(),
        ..Meta::default()
    };
    let mut files = Vec::new();
    if let Some(dir) = find_conversation(&paths.home, id) {
        let base = paths.home.join(".claude/projects").join(&dir);
        files.push((
            format!("claude/projects/{dir}/{id}.jsonl"),
            std::fs::read(base.join(format!("{id}.jsonl")))?,
        ));
        // What Claude Code keeps beside it: subagents, long tool results.
        let side = base.join(id);
        if side.is_dir() {
            for (rel, bytes) in read_tree(&side)? {
                files.push((format!("claude/projects/{dir}/{id}/{rel}"), bytes));
            }
        }
        meta.conversation_dir = Some(dir);
    }
    if let Ok(todos) = std::fs::read_dir(paths.home.join(".claude/todos")) {
        for t in todos.flatten() {
            let name = t.file_name().to_string_lossy().into_owned();
            if name.starts_with(id.as_str()) && t.path().is_file() {
                files.push((format!("claude/todos/{name}"), std::fs::read(t.path())?));
            }
        }
    }
    let mut bundles = Vec::new();
    for (dir, path) in repos(paths, spec) {
        match work_of(&paths.home, &path, spec.session_id.as_str()).await {
            Ok(Some((mut repo, bundle))) => {
                repo.dir = dir;
                repo.bundle = format!("repos/{}.bundle", bundles.len());
                bundles.push((repo.bundle.clone(), bundle));
                meta.repos.push(repo);
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("saving the work in {}: {e:#}", path.display()),
        }
    }
    Ok(Parts {
        meta,
        files,
        bundles,
    })
}

/// Every file under `dir`, by its path from there.
fn read_tree(dir: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)?.flatten() {
            let p = e.path();
            let kind = e.file_type()?;
            if kind.is_dir() {
                stack.push(p);
            } else if kind.is_file() {
                let rel = p
                    .strip_prefix(dir)
                    .map(|r| r.display().to_string())
                    .unwrap_or_default();
                out.push((rel, std::fs::read(&p)?));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Packs a snapshot into a gzipped tar: its size.
fn pack(parts: &Parts, to: &Path) -> Result<u64> {
    let file = std::fs::File::create(to)?;
    let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);
    let mut add = |name: &str, bytes: &[u8]| -> Result<()> {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append_data(&mut header, name, bytes)?;
        Ok(())
    };
    add("meta.json", &serde_json::to_vec_pretty(&parts.meta)?)?;
    for (name, bytes) in parts.files.iter().chain(parts.bundles.iter()) {
        add(name, bytes)?;
    }
    let gz = tar.into_inner()?;
    let mut file = gz.finish()?;
    file.flush()?;
    Ok(std::fs::metadata(to)?.len())
}

/// The size a packed snapshot of `packed` bytes takes once sealed.
fn sealed_size(packed: u64) -> u64 {
    let segments = packed.div_ceil(SEGMENT as u64).max(1);
    HEADER as u64 + packed + segments * TAG as u64
}

/// Seals the packed snapshot at `from` into `to` with `key`: its SHA-256.
fn seal_file(key: &[u8], from: &Path, to: &Path) -> Result<String> {
    let (mut sealer, header) = Sealer::new(key).map_err(|e| anyhow!("{e}"))?;
    let mut input = std::fs::File::open(from)?;
    let mut out = std::io::BufWriter::new(std::fs::File::create(to)?);
    let mut sha = Sha256::new();
    out.write_all(&header)?;
    sha.update(&header);
    let mut buf = vec![0u8; SEGMENT];
    let mut next = read_full(&mut input, &mut buf)?;
    loop {
        let chunk = buf.get(..next).unwrap_or_default().to_vec();
        let following = read_full(&mut input, &mut buf)?;
        let last = following == 0;
        let sealed = sealer.seal(&chunk, last).map_err(|e| anyhow!("{e}"))?;
        out.write_all(&sealed)?;
        sha.update(&sealed);
        if last {
            break;
        }
        next = following;
    }
    out.flush()?;
    Ok(hex::encode(sha.finalize()))
}

/// Reads until `buf` is full or the input ends: how much it read.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(buf.get_mut(n..).unwrap_or_default())? {
            0 => break,
            m => n += m,
        }
    }
    Ok(n)
}

/// A snapshot saved to a file, ready to hand to the host.
pub struct Saved {
    pub file: PathBuf,
    pub saved: SnapshotSaved,
}

/// Saves a snapshot of the session into `dir`, sealed with `key`, within
/// `max_bytes`: its conversation, then, to stay within the limit, without
/// the conversation's images, then without the uncommitted work. `None`
/// when there is nothing to save, or it doesn't fit even then.
pub async fn save(
    paths: &Paths,
    spec: &SessionSpec,
    key: &[u8],
    max_bytes: u64,
    claude_version: Option<&str>,
    agent_seq: u64,
    dir: &Path,
) -> Result<Option<Saved>> {
    let mut parts = collect(paths, spec, claude_version).await?;
    if parts.meta.conversation_dir.is_none() && parts.bundles.is_empty() {
        return Ok(None);
    }
    std::fs::create_dir_all(dir)?;
    let packed = dir.join("snapshot.tar.gz");
    let sealed = dir.join("snapshot.sealed");
    let mut left_out = Vec::new();
    let mut size = sealed_size(pack(&parts, &packed)?);
    if size > max_bytes && parts.meta.conversation_dir.is_some() {
        let mut images = 0;
        for (name, bytes) in parts.files.iter_mut() {
            if name.ends_with(".jsonl")
                && let Ok(text) = std::str::from_utf8(bytes)
            {
                let (stripped, n) = snap::without_images(text);
                if n > 0 {
                    images += n;
                    *bytes = stripped.into_bytes();
                }
            }
        }
        if images > 0 {
            parts.meta.images_left_out = images;
            left_out.push("images".to_owned());
            size = sealed_size(pack(&parts, &packed)?);
        }
    }
    if size > max_bytes && !parts.bundles.is_empty() && parts.meta.conversation_dir.is_some() {
        parts.bundles.clear();
        parts.meta.repos.clear();
        left_out.push("changes".to_owned());
        size = sealed_size(pack(&parts, &packed)?);
    }
    if size > max_bytes {
        let _ = std::fs::remove_file(&packed);
        bail!(
            "it would take {}, more than the {} the instance allows",
            snap::size_words(size),
            snap::size_words(max_bytes)
        );
    }
    let sha256 = seal_file(key, &packed, &sealed)?;
    let _ = std::fs::remove_file(&packed);
    Ok(Some(Saved {
        saved: SnapshotSaved {
            size: std::fs::metadata(&sealed)?.len(),
            sha256,
            claude_version: claude_version.map(str::to_owned),
            agent_seq,
            conversation: parts.meta.conversation_dir.is_some(),
            changes: !parts.meta.repos.is_empty(),
            left_out,
        },
        file: sealed,
    }))
}

// ───────────────────────────── restoring ─────────────────────────────

/// Reads a sealed snapshot's plaintext, segment by segment.
struct Unseal<R: Read> {
    input: R,
    opener: Opener,
    /// The sealed segment after the one being read, to tell the last.
    ahead: Vec<u8>,
    plain: Vec<u8>,
    pos: usize,
    done: bool,
}

impl<R: Read> Unseal<R> {
    fn new(mut input: R, key: &[u8]) -> Result<Self> {
        let mut header = [0u8; HEADER];
        input
            .read_exact(&mut header)
            .context("reading the snapshot")?;
        let opener = Opener::new(key, &header).map_err(|e| anyhow!("{e}"))?;
        let mut first = vec![0u8; SEGMENT + TAG];
        let n = read_full(&mut input, &mut first)?;
        first.truncate(n);
        Ok(Self {
            input,
            opener,
            ahead: first,
            plain: Vec::new(),
            pos: 0,
            done: false,
        })
    }
}

impl<R: Read> Read for Unseal<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.plain.len() {
            if self.done {
                return Ok(0);
            }
            let mut next = vec![0u8; SEGMENT + TAG];
            let n = read_full(&mut self.input, &mut next)?;
            next.truncate(n);
            let segment = std::mem::replace(&mut self.ahead, next);
            let last = self.ahead.is_empty();
            self.plain = self
                .opener
                .open(&segment, last)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            self.pos = 0;
            self.done = last;
        }
        let n = out.len().min(self.plain.len() - self.pos);
        if let (Some(dst), Some(src)) = (out.get_mut(..n), self.plain.get(self.pos..self.pos + n)) {
            dst.copy_from_slice(src);
        }
        self.pos += n;
        Ok(n)
    }
}

/// The SHA-256 of a file, hex.
pub fn sha256_of(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut sha = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        match f.read(&mut buf)? {
            0 => break,
            n => sha.update(buf.get(..n).unwrap_or_default()),
        }
    }
    Ok(hex::encode(sha.finalize()))
}

/// What a restore put back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Restored {
    pub conversation: bool,
    /// The repositories whose work is back in their working tree.
    pub changes: Vec<String>,
    /// The repositories whose branch moved on since, whose work is at
    /// `refs/fctl/wip/<session>`.
    pub moved: Vec<String>,
    /// Images the snapshot had left out of the conversation.
    pub images_left_out: usize,
}

/// Unpacks the sealed snapshot at `file` into `into`: its `meta.json`.
fn unpack(file: &Path, key: &[u8], into: &Path) -> Result<Meta> {
    let reader = Unseal::new(std::io::BufReader::new(std::fs::File::open(file)?), key)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(reader));
    std::fs::create_dir_all(into)?;
    // `unpack` keeps every entry inside `into`.
    tar.unpack(into).context("unpacking the snapshot")?;
    let meta: Meta = serde_json::from_slice(&std::fs::read(into.join("meta.json"))?)
        .context("reading the snapshot's meta.json")?;
    Ok(meta)
}

/// Makes `path`, and what is in it, the session's user's, where fctl-agent
/// runs as root.
fn give_to_user(path: &Path) {
    #[cfg(target_os = "linux")]
    if let Some(user) = crate::user::protected() {
        let mut stack = vec![path.to_path_buf()];
        while let Some(p) = stack.pop() {
            let _ = std::os::unix::fs::lchown(&p, Some(user.uid), Some(user.gid));
            if p.is_dir()
                && let Ok(entries) = std::fs::read_dir(&p)
            {
                stack.extend(entries.flatten().map(|e| e.path()));
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = path;
}

/// Copies what `from` holds into `to`, making the directories on the way,
/// for the session's user.
fn copy_into(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)?.flatten() {
            copy_into(&e.path(), &to.join(e.file_name()))?;
        }
    } else {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to)?;
    }
    give_to_user(to);
    Ok(())
}

/// Restores the sealed snapshot at `file` into this sandbox: checks it,
/// puts the conversation where Claude Code finds it for the working
/// directory, and each repository's work back. `claude_version`: the
/// Claude Code here, which must read what the snapshot's wrote.
pub async fn restore(
    paths: &Paths,
    spec: &SessionSpec,
    file: &Path,
    key: &[u8],
    sha256: &str,
    claude_version: Option<&str>,
    scratch: &Path,
) -> Result<Restored, String> {
    let have = sha256_of(file).map_err(|e| format!("reading it failed: {e:#}"))?;
    if !have.eq_ignore_ascii_case(sha256) {
        return Err("it came damaged: its checksum doesn't match".into());
    }
    let into = scratch.join("restore");
    let _ = std::fs::remove_dir_all(&into);
    let meta = {
        let (file, key, into) = (file.to_path_buf(), key.to_vec(), into.clone());
        tokio::task::spawn_blocking(move || unpack(&file, &key, &into))
            .await
            .map_err(|e| format!("unpacking it failed: {e}"))?
            .map_err(|e| format!("it doesn't open: {e:#}"))?
    };
    if meta.claude_session_id != spec.claude_session_id {
        return Err("it is another conversation's".into());
    }
    if let Some(why) = snap::unreadable_by(meta.claude_version.as_deref(), claude_version) {
        return Err(why);
    }
    let mut restored = Restored {
        images_left_out: meta.images_left_out,
        ..Restored::default()
    };
    let id = &meta.claude_session_id;
    if let Some(saved_dir) = &meta.conversation_dir {
        let here = conversation_dir(&paths.workdir);
        let projects = paths.home.join(".claude/projects");
        let from = into.join("claude/projects").join(saved_dir);
        copy_into(&from, &projects.join(&here))
            .map_err(|e| format!("putting the conversation back failed: {e:#}"))?;
        give_to_user(&paths.home.join(".claude"));
        restored.conversation = projects.join(&here).join(format!("{id}.jsonl")).is_file();
        let todos = into.join("claude/todos");
        if todos.is_dir() {
            let _ = copy_into(&todos, &paths.home.join(".claude/todos"));
        }
    }
    let session = spec.session_id.as_str();
    for repo in &meta.repos {
        let path = if repo.dir.is_empty() {
            paths.workdir.clone()
        } else {
            paths.home.join(&repo.dir)
        };
        let name = if repo.dir.is_empty() {
            spec.project
                .repo
                .as_ref()
                .map_or_else(|| "the working directory".to_owned(), |r| r.full_name())
        } else {
            repo.dir.trim_start_matches("repos/").to_owned()
        };
        match put_back(&paths.home, &path, &into.join(&repo.bundle), repo, session).await {
            Ok(true) => restored.changes.push(name),
            Ok(false) => restored.moved.push(name),
            Err(e) => tracing::warn!("putting the work back in {}: {e:#}", path.display()),
        }
    }
    let _ = std::fs::remove_dir_all(&into);
    Ok(restored)
}

/// Puts a repository's work back: fetches its bundle, and, where the
/// branch is where the snapshot left it or behind it, checks the work out
/// and resets to the commit before it, so that what was uncommitted is
/// again. False when the branch moved on elsewhere: the work stays at its
/// ref.
async fn put_back(
    home: &Path,
    repo: &Path,
    bundle: &Path,
    meta: &RepoMeta,
    session: &str,
) -> Result<bool> {
    if !repo.join(".git").exists() {
        bail!("{} isn't a repository here", repo.display());
    }
    // Where the session's user reads it.
    let here = repo.join(".git/fctl-snapshot.bundle");
    std::fs::copy(bundle, &here)?;
    give_to_user(&here);
    let r = wip_ref(session);
    let fetched = git(
        home,
        repo,
        &[
            "fetch",
            "--no-tags",
            &here.display().to_string(),
            &format!("+{r}:{r}"),
        ],
        &[],
    )
    .await;
    let _ = std::fs::remove_file(&here);
    fetched?;
    // The branch as cloned must be where the snapshot left it, or behind.
    let behind = git(
        home,
        repo,
        &["merge-base", "--is-ancestor", "HEAD", &meta.head],
        &[],
    )
    .await
    .is_ok();
    if !behind {
        return Ok(false);
    }
    git(home, repo, &["reset", "--hard", &meta.wip], &[]).await?;
    git(home, repo, &["reset", "--mixed", &meta.head], &[]).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn sh(dir: &Path, script: &str) -> Result<String> {
        let out = tokio::process::Command::new("sh")
            .args(["-c", script])
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .await?;
        if !out.status.success() {
            bail!("{script}: {}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    fn spec(id: &str) -> Result<SessionSpec> {
        let mut spec = crate::workspace::tests::spec()?;
        spec.claude_session_id = id.into();
        Ok(spec)
    }

    /// A repository's work as uncommitted: what changed and isn't staged,
    /// what is staged, and the untracked files.
    async fn work(repo: &Path) -> Result<(String, String, String)> {
        Ok((
            sh(repo, "git diff --name-status").await?,
            sh(repo, "git diff --cached --name-status").await?,
            sh(repo, "git ls-files --others --exclude-standard").await?,
        ))
    }

    fn expected() -> (String, String, String) {
        (
            "D\tgone.txt\nM\tkept.txt".into(),
            String::new(),
            "new.txt".into(),
        )
    }

    /// A sandbox with a clone of `origin`: its home and working directory.
    async fn sandbox(root: &Path, name: &str, origin: &Path) -> Result<Paths> {
        let home = root.join(name);
        std::fs::create_dir_all(&home)?;
        sh(
            &home,
            &format!("git clone -q {} workspace", origin.display()),
        )
        .await?;
        let state = home.join(".state");
        Ok(Paths::new(&home.join("workspace"), &home, &state))
    }

    /// A session saved in one sandbox resumes in a fresh clone with its
    /// conversation, its commits not pushed and its uncommitted work, as
    /// uncommitted work: changed, new and deleted files.
    #[tokio::test]
    async fn a_snapshot_carries_the_conversation_and_the_work_to_a_fresh_clone() -> Result<()> {
        let root = tempfile::tempdir()?;
        let origin = root.path().join("origin.git");
        sh(root.path(), "git init -q --bare -b main origin.git").await?;
        let seed = root.path().join("seed");
        std::fs::create_dir_all(&seed)?;
        sh(
            &seed,
            &format!(
                "git init -q -b main && echo one > kept.txt && echo gone > gone.txt && git add . && git commit -qm first && git push -q {} main",
                origin.display()
            ),
        )
        .await?;
        let id = "4f8e2a8c-0000-4000-8000-000000000001";
        let s = spec(id)?;
        let a = sandbox(root.path(), "a", &origin).await?;
        // Claude's conversation, with what it keeps beside it.
        let conv = a
            .home
            .join(".claude/projects")
            .join(conversation_dir(&a.workdir));
        std::fs::create_dir_all(conv.join(id).join("tool-results"))?;
        std::fs::write(
            conv.join(format!("{id}.jsonl")),
            "{\"type\":\"user\",\"message\":{\"content\":\"remember banana\"}}\n",
        )?;
        std::fs::write(conv.join(id).join("tool-results/x.txt"), "long output")?;
        // A commit not pushed, then work not committed.
        sh(
            &a.workdir,
            "echo two > local.txt && git add local.txt && git commit -qm local && echo changed > kept.txt && rm gone.txt && echo new > new.txt",
        )
        .await?;
        let key = [9u8; 32];
        let saved = save(
            &a,
            &s,
            &key,
            10_000_000,
            Some("2.1.140"),
            42,
            &root.path().join("out"),
        )
        .await?
        .ok_or_else(|| anyhow!("nothing saved"))?;
        assert!(saved.saved.conversation && saved.saved.changes);
        assert_eq!((saved.saved.agent_seq, saved.saved.left_out.len()), (42, 0));
        // The sandbox it was saved in is as it was.
        assert_eq!(work(&a.workdir).await?, expected());
        assert_eq!(sh(&a.workdir, "git log --format=%s -1").await?, "local");
        let b = sandbox(root.path(), "b", &origin).await?;
        let restored = restore(
            &b,
            &s,
            &saved.file,
            &key,
            &saved.saved.sha256,
            Some("2.1.141"),
            &root.path().join("scratch"),
        )
        .await
        .map_err(|e| anyhow!(e))?;
        assert!(restored.conversation);
        assert_eq!(restored.changes.len(), 1);
        let conv_b = b
            .home
            .join(".claude/projects")
            .join(conversation_dir(&b.workdir));
        assert!(std::fs::read_to_string(conv_b.join(format!("{id}.jsonl")))?.contains("banana"));
        assert_eq!(
            std::fs::read_to_string(conv_b.join(id).join("tool-results/x.txt"))?,
            "long output"
        );
        assert_eq!(sh(&b.workdir, "git log --format=%s -1").await?, "local");
        assert_eq!(
            std::fs::read_to_string(b.workdir.join("kept.txt"))?,
            "changed\n"
        );
        assert!(!b.workdir.join("gone.txt").exists());
        assert_eq!(work(&b.workdir).await?, expected());
        // Another key, a damaged file, an older Claude Code: refused.
        let scratch = root.path().join("scratch2");
        assert!(
            restore(
                &b,
                &s,
                &saved.file,
                &[1; 32],
                &saved.saved.sha256,
                None,
                &scratch
            )
            .await
            .is_err()
        );
        assert!(
            restore(&b, &s, &saved.file, &key, "00", None, &scratch)
                .await
                .is_err()
        );
        let older = restore(
            &b,
            &s,
            &saved.file,
            &key,
            &saved.saved.sha256,
            Some("2.1.100"),
            &scratch,
        )
        .await;
        assert!(older.is_err_and(|e| e.contains("2.1.140")));
        Ok(())
    }

    /// Work whose branch moved on elsewhere stays at its ref, and a
    /// snapshot over the limit leaves the work out, then gives up.
    #[tokio::test]
    async fn work_on_a_branch_that_moved_stays_aside_and_limits_hold() -> Result<()> {
        let root = tempfile::tempdir()?;
        let origin = root.path().join("origin.git");
        sh(root.path(), "git init -q --bare -b main origin.git").await?;
        let seed = root.path().join("seed");
        std::fs::create_dir_all(&seed)?;
        sh(
            &seed,
            &format!(
                "git init -q -b main && echo one > a.txt && git add . && git commit -qm first && git push -q {} main",
                origin.display()
            ),
        )
        .await?;
        let id = "4f8e2a8c-0000-4000-8000-000000000002";
        let s = spec(id)?;
        let a = sandbox(root.path(), "a", &origin).await?;
        sh(&a.workdir, "echo mine > a.txt").await?;
        let key = [3u8; 32];
        let saved = save(&a, &s, &key, 10_000_000, None, 1, &root.path().join("out"))
            .await?
            .ok_or_else(|| anyhow!("nothing saved"))?;
        // Someone pushes to the branch meanwhile.
        sh(
            &seed,
            &format!(
                "echo theirs > b.txt && git add . && git commit -qm theirs && git push -q {} main",
                origin.display()
            ),
        )
        .await?;
        let b = sandbox(root.path(), "b", &origin).await?;
        let restored = restore(
            &b,
            &s,
            &saved.file,
            &key,
            &saved.saved.sha256,
            None,
            &root.path().join("scratch"),
        )
        .await
        .map_err(|e| anyhow!(e))?;
        assert_eq!((restored.changes.len(), restored.moved.len()), (0, 1));
        assert_eq!(std::fs::read_to_string(b.workdir.join("a.txt"))?, "one\n");
        let kept = sh(
            &b.workdir,
            &format!("git show {}:a.txt", wip_ref(s.session_id.as_str())),
        )
        .await?;
        assert_eq!(kept, "mine");
        // Nothing to save: no conversation, nothing changed.
        let c = sandbox(root.path(), "c", &origin).await?;
        assert!(
            save(&c, &s, &key, 10_000_000, None, 1, &root.path().join("out3"))
                .await?
                .is_none()
        );
        // Over the limit with work alone: nothing fits.
        let big = save(&a, &s, &key, 10, None, 1, &root.path().join("out2")).await;
        assert!(big.is_err());
        Ok(())
    }
}
