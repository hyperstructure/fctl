//! Sandbox images built from a project's repository: a Dockerfile, or a dev
//! container configuration, as the project's default branch has it. A host
//! builds one when its owner allows builds from the repository
//! (`allowed_builds` in host.toml), keeps it, and builds it again once the
//! files it comes from change. One build runs at a time, within
//! `build_timeout_minutes`, and an image larger than `max_build_mb` is
//! removed.

use crate::session::HostCtx;
use anyhow::{Context, Result, bail};
use fctl_proto::runner::SessionSpec;
use fctl_proto::session::{ImageBuild, RepoRef};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// Changes when fctl builds differently, so that hosts build again.
const RECIPE: &str = "fctl image build 1";

/// The largest repository archive a host downloads to build from.
const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// How long downloading the archive may take.
const ARCHIVE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// What to build, as the setting and the file it names say. Paths are in
/// the repository, `""` for its root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recipe {
    /// Build `dockerfile` in `context`.
    Build {
        dockerfile: String,
        context: String,
        args: BTreeMap<String, String>,
        target: Option<String>,
    },
    /// Start from this image: a dev container configuration that names one
    /// and builds nothing.
    Image(String),
}

/// The recipe for `setting`, given the text of the file it names, and what
/// fctl leaves out of a dev container configuration, for the log.
pub fn recipe(setting: &ImageBuild, file: &[u8]) -> Result<(Recipe, Vec<String>)> {
    if !setting.is_devcontainer() {
        let recipe = Recipe::Build {
            dockerfile: setting.file.clone(),
            context: setting.context.clone().unwrap_or_default(),
            args: BTreeMap::new(),
            target: None,
        };
        return Ok((recipe, vec![]));
    }
    let text = String::from_utf8_lossy(file);
    let config: Value = serde_json::from_str(&strip_jsonc(&text))
        .with_context(|| format!("reading {} (JSON with comments)", setting.file))?;
    devcontainer(&setting.file, &config)
}

/// The recipe in a dev container configuration at `path`: its `image`, or
/// its `build` (`dockerfile`, `context`, `args`, `target`; `dockerFile` and
/// `context` at the top level in older ones), with paths relative to it.
fn devcontainer(path: &str, config: &Value) -> Result<(Recipe, Vec<String>)> {
    if config.get("dockerComposeFile").is_some() {
        bail!(
            "{path} uses Docker Compose, which fctl doesn't build: name an image or a Dockerfile in it, or a Dockerfile in the project's settings"
        );
    }
    let dir = match path.rsplit_once('/') {
        Some((dir, _)) => dir,
        None => "",
    };
    let build = config.get("build");
    let field = |name: &str| {
        build
            .and_then(|b| b.get(name))
            .or_else(|| config.get(name))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
    };
    let dockerfile = field("dockerfile").or_else(|| field("dockerFile"));
    let mut notes = Vec::new();
    let ignored: Vec<&str> = [
        "features",
        "onCreateCommand",
        "updateContentCommand",
        "postCreateCommand",
        "postStartCommand",
        "postAttachCommand",
        "containerEnv",
        "mounts",
        "runArgs",
    ]
    .into_iter()
    .filter(|k| config.get(*k).is_some_and(|v| !is_empty(v)))
    .collect();
    if !ignored.is_empty() {
        notes.push(format!(
            "fctl leaves out {} from {path}: install what they would in the Dockerfile, or in the project's setup script",
            ignored.join(", ")
        ));
    }
    if let Some(user) = ["remoteUser", "containerUser"]
        .iter()
        .find_map(|k| config.get(*k).and_then(Value::as_str))
    {
        notes.push(format!(
            "Sessions run as the image's own user, not {user} from {path}: end the Dockerfile with `USER {user}` to run as {user}"
        ));
    }
    let Some(dockerfile) = dockerfile else {
        return match config.get("image").and_then(Value::as_str) {
            Some(image) if !image.trim().is_empty() => {
                Ok((Recipe::Image(image.trim().to_owned()), notes))
            }
            _ => bail!("{path} names neither an image nor a Dockerfile"),
        };
    };
    let context = field("context").unwrap_or(".");
    let args = build
        .and_then(|b| b.get("args"))
        .and_then(Value::as_object)
        .map(|args| {
            args.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let target = build
        .and_then(|b| b.get("target"))
        .and_then(Value::as_str)
        .filter(|t| !t.trim().is_empty())
        .map(str::to_owned);
    let recipe = Recipe::Build {
        dockerfile: join(dir, dockerfile)
            .with_context(|| format!("the Dockerfile {path} names"))?,
        context: join(dir, context).with_context(|| format!("the context {path} names"))?,
        args,
        target,
    };
    Ok((recipe, notes))
}

fn is_empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Object(o) => o.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::String(s) => s.trim().is_empty(),
        _ => false,
    }
}

/// `rel` from the directory `dir`, both in the repository: `""` for its
/// root. Fails for a path that leaves it.
fn join(dir: &str, rel: &str) -> Result<String> {
    let mut parts: Vec<&str> = if rel.starts_with('/') {
        vec![]
    } else {
        dir.split('/').filter(|p| !p.is_empty()).collect()
    };
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    bail!("`{rel}` is outside the repository");
                }
            }
            p => parts.push(p),
        }
    }
    Ok(parts.join("/"))
}

/// JSON with comments, as dev container configurations are, made plain:
/// without `//` and `/* */` comments, or commas before a closing bracket.
pub fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => {
                    if let Some(next) = chars.next() {
                        out.push(next);
                    }
                }
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
                out.push(' ');
            }
            (',', _) => {
                // A comma whose next significant character closes a bracket.
                let rest: String = chars.clone().collect();
                let next = strip_leading_comments(&rest);
                if !next.starts_with(['}', ']']) {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// `text` without the whitespace and comments it starts with.
fn strip_leading_comments(mut text: &str) -> &str {
    loop {
        let trimmed = text.trim_start();
        if let Some(rest) = trimmed.strip_prefix("//") {
            text = rest.split_once('\n').map_or("", |(_, r)| r);
        } else if let Some(rest) = trimmed.strip_prefix("/*") {
            text = rest.split_once("*/").map_or("", |(_, r)| r);
        } else {
            return trimmed;
        }
    }
}

/// What a build's tag comes from: the recipe, the text of the file the
/// setting names, the Dockerfile's text, and the git tree of the context,
/// which changes with any file in it. A host builds again when one changes.
pub fn key(recipe: &Recipe, file: &[u8], dockerfile: &[u8], context_tree: &str) -> String {
    let mut h = Sha256::new();
    let mut part = |bytes: &[u8]| {
        h.update((bytes.len() as u64).to_be_bytes());
        h.update(bytes);
    };
    part(RECIPE.as_bytes());
    part(file);
    part(dockerfile);
    part(context_tree.as_bytes());
    if let Recipe::Build {
        dockerfile,
        context,
        args,
        target,
    } = recipe
    {
        part(dockerfile.as_bytes());
        part(context.as_bytes());
        for (k, v) in args {
            part(k.as_bytes());
            part(v.as_bytes());
        }
        part(target.as_deref().unwrap_or_default().as_bytes());
    }
    hex::encode(h.finalize())[..12].to_owned()
}

/// The name of a project's builds on this host: `fctl-build-prj_…`.
pub fn repository(project_id: &str) -> String {
    let id: String = project_id
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("fctl-build-{}", id.trim_matches(['-', '_']))
}

/// The GitHub API, with a session's token.
pub struct Github {
    http: fctl_client::HttpClient,
    base: String,
    token: String,
}

impl Github {
    pub fn new(base: &str, token: &str) -> Self {
        Self {
            http: fctl_client::HttpClient::new(Duration::from_secs(60)),
            base: base.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
        }
    }

    fn url(&self, repo: &RepoRef, rest: &str) -> String {
        format!(
            "{}/repos/{}/{}/{rest}",
            self.base,
            encode(&repo.owner),
            encode(&repo.name)
        )
    }

    /// GET `url`; `None` for 404.
    async fn get(&self, url: &str, accept: &str) -> Result<Option<Vec<u8>>> {
        let headers = [("accept", accept), ("x-github-api-version", "2022-11-28")];
        let (status, _, body) = self
            .http
            .get_with(url, Some(&self.token), &headers)
            .await
            .with_context(|| format!("asking GitHub ({url})"))?;
        if status.as_u16() == 404 {
            return Ok(None);
        }
        if !status.is_success() {
            let text: String = String::from_utf8_lossy(&body).chars().take(300).collect();
            bail!("GitHub answered {status} for {url}: {text}");
        }
        Ok(Some(body.to_vec()))
    }

    /// The commit at the tip of `branch`.
    pub async fn commit(&self, repo: &RepoRef, branch: &str) -> Result<String> {
        let url = self.url(repo, &format!("commits/{}", encode_path(branch)));
        let body = self
            .get(&url, "application/vnd.github.sha")
            .await?
            .with_context(|| format!("{} has no branch {branch}", repo.full_name()))?;
        let sha = String::from_utf8_lossy(&body).trim().to_owned();
        if sha.len() < 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("GitHub sent no commit for {branch}");
        }
        Ok(sha)
    }

    /// The file at `path` in `commit`, or `None`.
    pub async fn file(&self, repo: &RepoRef, commit: &str, path: &str) -> Result<Option<Vec<u8>>> {
        let url = self.url(
            repo,
            &format!("contents/{}?ref={commit}", encode_path(path)),
        );
        self.get(&url, "application/vnd.github.raw").await
    }

    /// The git tree of the directory `dir` in `commit` (`""`: the root), or
    /// `None` when there is no such directory.
    pub async fn tree(&self, repo: &RepoRef, commit: &str, dir: &str) -> Result<Option<String>> {
        let url = self.url(repo, &format!("git/trees/{commit}:{}", encode_path(dir)));
        let Some(body) = self.get(&url, "application/vnd.github+json").await? else {
            return Ok(None);
        };
        let v: Value = serde_json::from_slice(&body).context("reading GitHub's tree")?;
        Ok(v.get("sha").and_then(Value::as_str).map(str::to_owned))
    }

    /// Downloads the repository as it is in `commit`, a `.tar.gz`, to `dest`.
    pub async fn archive(&self, repo: &RepoRef, commit: &str, dest: &Path) -> Result<()> {
        let url = self.url(repo, &format!("tarball/{commit}"));
        let headers = [("x-github-api-version", "2022-11-28")];
        self.http
            .download_to(
                &url,
                Some(&self.token),
                &headers,
                dest,
                MAX_ARCHIVE_BYTES,
                ARCHIVE_TIMEOUT,
            )
            .await
            .with_context(|| format!("downloading {} at {commit}", repo.full_name()))?;
        Ok(())
    }
}

/// Percent-encodes one part of a URL's path.
fn encode(part: &str) -> String {
    let mut out = String::new();
    for b in part.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Percent-encodes a path, keeping its slashes.
fn encode_path(path: &str) -> String {
    path.split('/').map(encode).collect::<Vec<_>>().join("/")
}

/// Unpacks the parts of a GitHub archive (`.tar.gz`, one directory at the
/// top) that a build reads, the files under `context` and `dockerfile`,
/// into `dest`, as they are in the repository. Only files, directories and
/// symbolic links; nothing outside `dest`.
pub fn unpack(archive: &Path, dest: &Path, context: &str, dockerfile: &str) -> Result<()> {
    let file =
        std::fs::File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    tar.set_preserve_permissions(true);
    let context = Path::new(context);
    let dockerfile = Path::new(dockerfile);
    std::fs::create_dir_all(dest)?;
    let root = dest.canonicalize()?;
    for entry in tar.entries().context("reading the repository's archive")? {
        let mut entry = entry.context("reading the repository's archive")?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir() || kind.is_symlink()) {
            continue;
        }
        let path = entry.path()?.into_owned();
        // The archive's own directory, `owner-repo-sha/`, comes first.
        let mut parts = path.components();
        parts.next();
        let rel: PathBuf = parts.collect();
        if rel.as_os_str().is_empty()
            || !rel.components().all(|c| matches!(c, Component::Normal(_)))
        {
            continue;
        }
        let wanted = rel.starts_with(context)
            || rel == dockerfile
            || (kind.is_dir() && (context.starts_with(&rel) || dockerfile.starts_with(&rel)));
        if !wanted {
            continue;
        }
        let out = dest.join(&rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
            // Never through a link out of `dest`.
            if !parent.canonicalize()?.starts_with(&root) {
                continue;
            }
        }
        entry
            .unpack(&out)
            .with_context(|| format!("unpacking {}", rel.display()))?;
    }
    Ok(())
}

/// The last build of each project on this host, in `builds.json` in its
/// data directory.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Builds {
    pub projects: BTreeMap<String, Built>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Built {
    /// The image.
    pub tag: String,
    /// The commit it was built from.
    pub commit: String,
}

impl Builds {
    fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("builds.json")
    }

    pub fn load(data_dir: &Path) -> Self {
        std::fs::read_to_string(Self::path(data_dir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, data_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(data_dir)?;
        let path = Self::path(data_dir);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// What a project's build resolves to at the tip of its default branch.
enum Resolved {
    /// The image to start from, built or not.
    Ready(String),
    /// Build `tag` from `commit`.
    Build {
        tag: String,
        commit: String,
        recipe: Recipe,
    },
}

/// The image a session of a project that builds its own starts from: this
/// host's build of the files on the project's default branch, built now
/// when they changed since. When GitHub can't be asked, the last build.
pub async fn image(
    ctx: &HostCtx,
    spec: &SessionSpec,
    log: &(dyn Fn(String) + Send + Sync),
) -> Result<String> {
    let project = &spec.project;
    let setting = project
        .settings
        .image_build
        .as_ref()
        .context("the project builds no image")?;
    let repo = project
        .repo
        .as_ref()
        .context("the project has no repository to build its image from")?;
    let token = spec
        .credentials
        .github
        .as_ref()
        .or(spec.credentials.github_read.as_ref())
        .map(|c| c.token.as_str())
        .with_context(|| format!("the session has no GitHub token for {}", repo.full_name()))?;
    let gh = Github::new(&ctx.cfg.github_api, token);
    let data_dir = ctx.cfg.data_dir();
    let id = project.id.as_str();
    let name = repository(id);
    let resolved = match resolve(&gh, repo, &project.default_branch, setting, &name, log).await {
        Ok(r) => r,
        Err(e) => {
            // GitHub may be down: the last build will do meanwhile.
            let last = Builds::load(&data_dir).projects.get(id).cloned();
            match last {
                Some(last) if ctx.driver.has_image(&last.tag).await => {
                    log(format!(
                        "Could not check {} for changes to its image ({e:#}); starting from the last build, {}",
                        repo.full_name(),
                        last.tag
                    ));
                    return Ok(last.tag);
                }
                _ => return Err(e),
            }
        }
    };
    let (tag, commit, recipe) = match resolved {
        Resolved::Ready(image) => return Ok(image),
        Resolved::Build {
            tag,
            commit,
            recipe,
        } => (tag, commit, recipe),
    };
    if !ctx.driver.has_image(&tag).await {
        // One build at a time; one that waited may find its image built.
        let _turn = ctx.builds.lock().await;
        if !ctx.driver.has_image(&tag).await {
            build(ctx, &gh, repo, &commit, &recipe, &tag, setting, log).await?;
        }
    }
    let mut builds = Builds::load(&data_dir);
    let previous = builds.projects.insert(
        id.to_owned(),
        Built {
            tag: tag.clone(),
            commit,
        },
    );
    if let Err(e) = builds.save(&data_dir) {
        tracing::warn!("saving builds.json: {e:#}");
    }
    // The project's earlier build goes, unless a sandbox still uses it.
    if let Some(old) = previous.filter(|old| old.tag != tag) {
        match ctx.driver.remove_image(&old.tag).await {
            Ok(()) => tracing::info!("removed {}, an earlier build", old.tag),
            Err(e) => tracing::info!("keeping {}: {e:#}", old.tag),
        }
    }
    Ok(tag)
}

/// Reads the setting's file at the tip of `branch` and works out what to
/// build, or the image to start from.
async fn resolve(
    gh: &Github,
    repo: &RepoRef,
    branch: &str,
    setting: &ImageBuild,
    name: &str,
    log: &(dyn Fn(String) + Send + Sync),
) -> Result<Resolved> {
    let commit = gh.commit(repo, branch).await?;
    let missing = |path: &str| {
        format!(
            "{path} isn't in {} on {branch} (Settings → Sandbox → Image)",
            repo.full_name()
        )
    };
    let file = gh
        .file(repo, &commit, &setting.file)
        .await?
        .with_context(|| missing(&setting.file))?;
    let (recipe, notes) = recipe(setting, &file)?;
    for note in notes {
        log(note);
    }
    let (dockerfile, context) = match &recipe {
        Recipe::Image(image) => return Ok(Resolved::Ready(image.clone())),
        Recipe::Build {
            dockerfile,
            context,
            ..
        } => (dockerfile, context),
    };
    let dockerfile_text = if *dockerfile == setting.file {
        file.clone()
    } else {
        gh.file(repo, &commit, dockerfile)
            .await?
            .with_context(|| missing(dockerfile))?
    };
    let tree = gh.tree(repo, &commit, context).await?.with_context(|| {
        format!(
            "{} has no directory {} on {branch} to build in",
            repo.full_name(),
            if context.is_empty() { "/" } else { context }
        )
    })?;
    let tag = format!("{name}:{}", key(&recipe, &file, &dockerfile_text, &tree));
    Ok(Resolved::Build {
        tag,
        commit,
        recipe,
    })
}

/// Downloads the repository at `commit` and builds `tag` from it.
#[allow(clippy::too_many_arguments)]
async fn build(
    ctx: &HostCtx,
    gh: &Github,
    repo: &RepoRef,
    commit: &str,
    recipe: &Recipe,
    tag: &str,
    setting: &ImageBuild,
    log: &(dyn Fn(String) + Send + Sync),
) -> Result<()> {
    let Recipe::Build {
        dockerfile,
        context,
        args,
        target,
    } = recipe
    else {
        return Ok(());
    };
    let short = &commit[..commit.len().min(7)];
    log(format!(
        "Building the sandbox image from {} in {} at {short}",
        setting.file,
        repo.full_name()
    ));
    let dir = ctx.cfg.data_dir().join("builds").join(format!(
        "{}-{}",
        tag.replace([':', '/'], "-"),
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let result = async {
        let archive = dir.join("repo.tar.gz");
        gh.archive(repo, commit, &archive).await?;
        let src = dir.join("src");
        let (archive2, src2, context2, dockerfile2) = (
            archive.clone(),
            src.clone(),
            context.clone(),
            dockerfile.clone(),
        );
        tokio::task::spawn_blocking(move || unpack(&archive2, &src2, &context2, &dockerfile2))
            .await
            .context("unpacking the repository")??;
        let _ = std::fs::remove_file(&archive);
        let limit = Duration::from_secs(u64::from(ctx.cfg.build_timeout_minutes.max(1)) * 60);
        let job = crate::driver::BuildJob {
            tag,
            dockerfile: &src.join(dockerfile),
            context: &src.join(context),
            args,
            target: target.as_deref(),
            limit,
        };
        ctx.driver.build(&job, log).await?;
        let max = u64::from(ctx.cfg.max_build_mb) * 1024 * 1024;
        if let Some(size) = ctx.driver.image_size(tag).await
            && size > max
        {
            let _ = ctx.driver.remove_image(tag).await;
            bail!(
                "The image is {}, more than the {} this host keeps (max_build_mb in host.toml)",
                fctl_domain::size::memory(size / 1024 / 1024),
                fctl_domain::size::memory(u64::from(ctx.cfg.max_build_mb))
            );
        }
        log(format!("Built {tag}"));
        anyhow::Ok(())
    }
    .await;
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn setting(file: &str, context: Option<&str>) -> ImageBuild {
        ImageBuild {
            file: file.into(),
            context: context.map(str::to_owned),
        }
    }

    fn build(dockerfile: &str, context: &str) -> Recipe {
        Recipe::Build {
            dockerfile: dockerfile.into(),
            context: context.into(),
            args: BTreeMap::new(),
            target: None,
        }
    }

    #[test]
    fn a_dockerfile_builds_in_its_context() -> Result<()> {
        let (r, notes) = recipe(&setting("docker/Dockerfile", None), b"FROM x")?;
        assert_eq!(r, build("docker/Dockerfile", ""));
        assert!(notes.is_empty());
        let (r, _) = recipe(&setting("Dockerfile", Some("app")), b"FROM x")?;
        assert_eq!(r, build("Dockerfile", "app"));
        Ok(())
    }

    #[test]
    fn reads_json_with_comments() -> Result<()> {
        let text = r#"{
  // The name.
  "name": "app // not a comment", /* a block
  comment */
  "build": {
    "dockerfile": "Dockerfile",
    "args": { "A": "1", },
  },
}"#;
        let v: Value = serde_json::from_str(&strip_jsonc(text))?;
        assert_eq!(v["name"], "app // not a comment");
        assert_eq!(v["build"]["args"]["A"], "1");
        assert_eq!(strip_jsonc(r#"{"a": "x\"," , }"#), r#"{"a": "x\","  }"#);
        assert_eq!(strip_jsonc("[1, // two\n]"), "[1 \n]");
        Ok(())
    }

    #[test]
    fn dev_containers_build_relative_to_their_configuration() -> Result<()> {
        let config = json!({
            "build": { "dockerfile": "Dockerfile", "context": "..", "args": { "V": "3" }, "target": "dev" },
            "features": { "ghcr.io/devcontainers/features/node:1": {} },
            "remoteUser": "vscode",
            "postCreateCommand": "npm ci"
        });
        let (r, notes) = devcontainer(".devcontainer/devcontainer.json", &config)?;
        assert_eq!(
            r,
            Recipe::Build {
                dockerfile: ".devcontainer/Dockerfile".into(),
                context: "".into(),
                args: BTreeMap::from([("V".to_owned(), "3".to_owned())]),
                target: Some("dev".into()),
            }
        );
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(
            notes[0].contains("features, postCreateCommand"),
            "{notes:?}"
        );
        assert!(notes[1].contains("USER vscode"), "{notes:?}");
        // The older form, and the default context: the configuration's directory.
        let (r, _) = devcontainer(
            ".devcontainer/devcontainer.json",
            &json!({ "dockerFile": "../docker/Dockerfile" }),
        )?;
        assert_eq!(r, build("docker/Dockerfile", ".devcontainer"));
        // An image, built from nothing.
        let (r, notes) = devcontainer(
            ".devcontainer.json",
            &json!({ "image": "mcr.microsoft.com/devcontainers/rust:1", "features": {} }),
        )?;
        assert_eq!(
            r,
            Recipe::Image("mcr.microsoft.com/devcontainers/rust:1".into())
        );
        assert!(notes.is_empty());
        // What fctl can't build.
        for config in [
            json!({ "dockerComposeFile": "compose.yml", "service": "app" }),
            json!({ "name": "nothing" }),
            json!({ "build": { "dockerfile": "../../Dockerfile" } }),
        ] {
            assert!(
                devcontainer(".devcontainer/devcontainer.json", &config).is_err(),
                "{config}"
            );
        }
        Ok(())
    }

    #[test]
    fn builds_again_when_what_they_come_from_changes() {
        let r = build("Dockerfile", "");
        let k = key(&r, b"FROM a", b"FROM a", "tree1");
        assert_eq!(k.len(), 12);
        assert_eq!(k, key(&r, b"FROM a", b"FROM a", "tree1"));
        assert_ne!(k, key(&r, b"FROM a", b"FROM a", "tree2"));
        assert_ne!(k, key(&r, b"FROM b", b"FROM a", "tree1"));
        assert_ne!(
            k,
            key(&build("Dockerfile", "app"), b"FROM a", b"FROM a", "tree1")
        );
        assert_eq!(
            repository("prj_01M57YPG1ATK9ARAXTZG43G51J"),
            "fctl-build-prj_01m57ypg1atk9araxtzg43g51j"
        );
    }

    #[test]
    fn joins_paths_in_the_repository() -> Result<()> {
        assert_eq!(
            join(".devcontainer", "Dockerfile")?,
            ".devcontainer/Dockerfile"
        );
        assert_eq!(join(".devcontainer", "..")?, "");
        assert_eq!(join(".devcontainer", "./x/../y")?, ".devcontainer/y");
        assert_eq!(join("", ".")?, "");
        assert!(join(".devcontainer", "../..").is_err());
        Ok(())
    }

    /// A GitHub archive of `files`, under `acme-app-1234567/`, with a
    /// symbolic link and a hard link.
    fn archive(dir: &Path, files: &[(&str, &str)]) -> Result<PathBuf> {
        let path = dir.join("repo.tar.gz");
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(&path)?,
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(gz);
        for (name, text) in files {
            let mut h = tar::Header::new_gnu();
            h.set_mode(0o755);
            h.set_size(text.len() as u64);
            tar.append_data(&mut h, format!("acme-app-1234567/{name}"), text.as_bytes())?;
        }
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Link);
        link.set_size(0);
        tar.append_link(&mut link, "acme-app-1234567/app/hard", "/etc/passwd")?;
        let mut sym = tar::Header::new_gnu();
        sym.set_entry_type(tar::EntryType::Symlink);
        sym.set_size(0);
        tar.append_link(&mut sym, "acme-app-1234567/app/link", "run.sh")?;
        tar.into_inner()?.finish()?;
        Ok(path)
    }

    #[test]
    fn unpacks_what_the_build_reads() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let path = archive(
            tmp.path(),
            &[
                ("README.md", "# app\n"),
                ("docker/Dockerfile", "FROM x\n"),
                ("app/run.sh", "#!/bin/sh\n"),
                ("app/src/main.rs", "fn main() {}\n"),
                ("web/index.html", "<p>\n"),
            ],
        )?;
        let dest = tmp.path().join("src");
        unpack(&path, &dest, "app", "docker/Dockerfile")?;
        assert!(dest.join("docker/Dockerfile").is_file());
        assert!(dest.join("app/src/main.rs").is_file());
        assert!(!dest.join("web").exists());
        assert!(!dest.join("README.md").exists());
        // Hard links could point anywhere; symbolic links stay links.
        assert!(!dest.join("app/hard").exists());
        assert!(std::fs::symlink_metadata(dest.join("app/link"))?.is_symlink());
        // The root context takes everything.
        let all = tmp.path().join("all");
        unpack(&path, &all, "", "docker/Dockerfile")?;
        assert!(all.join("README.md").is_file());
        assert!(all.join("web/index.html").is_file());
        Ok(())
    }

    #[test]
    fn remembers_the_last_build_of_each_project() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        assert_eq!(Builds::load(tmp.path()), Builds::default());
        let mut b = Builds::default();
        b.projects.insert(
            "prj_1".into(),
            Built {
                tag: "fctl-build-prj_1:abc".into(),
                commit: "f".repeat(40),
            },
        );
        b.save(tmp.path())?;
        assert_eq!(Builds::load(tmp.path()), b);
        Ok(())
    }

    use crate::driver::Driver;
    use crate::driver::container::Flavor;
    use crate::driver::container::fake::FakeCli;
    use crate::session::HostCtx;
    use std::sync::{Arc, Mutex, PoisonError};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A stand-in for GitHub with one repository, acme/app, and the files on
    /// its main branch, which tests change. Archives come from a second
    /// address, as GitHub's come from codeload.github.com.
    struct FakeGithub {
        url: String,
        repo: Arc<Mutex<FakeRepo>>,
    }

    #[derive(Default)]
    struct FakeRepo {
        files: BTreeMap<String, String>,
        commits: u32,
        down: bool,
        /// Each request: the path, and whether it carried the token.
        requests: Vec<(String, bool)>,
    }

    impl FakeRepo {
        fn commit(&self) -> String {
            format!("{:040x}", self.commits + 1)
        }

        /// A tree hash of the files under `dir`.
        fn tree(&self, dir: &str) -> Option<String> {
            let prefix = if dir.is_empty() {
                String::new()
            } else {
                format!("{dir}/")
            };
            let mut h = Sha256::new();
            let mut any = false;
            for (path, text) in &self.files {
                if let Some(rest) = path.strip_prefix(&prefix) {
                    any = true;
                    h.update(rest.as_bytes());
                    h.update(text.as_bytes());
                }
            }
            any.then(|| hex::encode(h.finalize()))
        }

        fn archive(&self) -> Result<Vec<u8>> {
            let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            let mut tar = tar::Builder::new(gz);
            for (path, text) in &self.files {
                let mut h = tar::Header::new_gnu();
                h.set_mode(0o644);
                h.set_size(text.len() as u64);
                let top = format!("acme-app-{}", &self.commit()[..7]);
                tar.append_data(&mut h, format!("{top}/{path}"), text.as_bytes())?;
            }
            Ok(tar.into_inner()?.finish()?)
        }
    }

    impl FakeGithub {
        async fn start(files: &[(&str, &str)]) -> Result<Self> {
            let api = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let codeload = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let url = format!("http://{}", api.local_addr()?);
            let codeload_url = format!("http://{}", codeload.local_addr()?);
            let repo = Arc::new(Mutex::new(FakeRepo {
                files: files
                    .iter()
                    .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
                    .collect(),
                ..Default::default()
            }));
            for listener in [api, codeload] {
                let (repo, codeload_url) = (repo.clone(), codeload_url.clone());
                tokio::spawn(async move {
                    while let Ok((mut sock, _)) = listener.accept().await {
                        let (repo, codeload_url) = (repo.clone(), codeload_url.clone());
                        tokio::spawn(async move {
                            let _ = serve(&mut sock, &repo, &codeload_url).await;
                        });
                    }
                });
            }
            Ok(Self { url, repo })
        }

        fn change(&self, f: impl FnOnce(&mut FakeRepo)) {
            let mut repo = self.repo.lock().unwrap_or_else(PoisonError::into_inner);
            f(&mut repo);
            repo.commits += 1;
        }

        fn requests(&self) -> Vec<(String, bool)> {
            let repo = self.repo.lock().unwrap_or_else(PoisonError::into_inner);
            repo.requests.clone()
        }
    }

    /// Answers one request, then closes the connection.
    async fn serve(
        sock: &mut tokio::net::TcpStream,
        repo: &Mutex<FakeRepo>,
        codeload: &str,
    ) -> Result<()> {
        let mut head = Vec::new();
        let mut buf = [0u8; 4096];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = sock.read(&mut buf).await?;
            if n == 0 {
                return Ok(());
            }
            head.extend_from_slice(&buf[..n]);
        }
        let head = String::from_utf8_lossy(&head).into_owned();
        let target = head
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_owned();
        let authorized = head
            .to_ascii_lowercase()
            .contains("authorization: bearer gh-token");
        let (status, location, body) = {
            let mut r = repo.lock().unwrap_or_else(PoisonError::into_inner);
            r.requests.push((target.clone(), authorized));
            route(&r, &target, codeload)?
        };
        let mut response = format!(
            "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n",
            body.len()
        );
        if let Some(l) = location {
            response.push_str(&format!("location: {l}\r\n"));
        }
        response.push_str("\r\n");
        sock.write_all(response.as_bytes()).await?;
        sock.write_all(&body).await?;
        sock.shutdown().await?;
        Ok(())
    }

    fn route(
        r: &FakeRepo,
        target: &str,
        codeload: &str,
    ) -> Result<(&'static str, Option<String>, Vec<u8>)> {
        let not_found = Ok(("404 Not Found", None, b"{}".to_vec()));
        if r.down {
            return Ok(("503 Service Unavailable", None, b"{}".to_vec()));
        }
        let commit = r.commit();
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        if path == "/repos/acme/app/commits/main" {
            return Ok(("200 OK", None, commit.into_bytes()));
        }
        if let Some(file) = path.strip_prefix("/repos/acme/app/contents/") {
            if query != format!("ref={commit}") {
                return not_found;
            }
            return match r.files.get(file) {
                Some(text) => Ok(("200 OK", None, text.clone().into_bytes())),
                None => not_found,
            };
        }
        if let Some(rest) = path.strip_prefix(&format!("/repos/acme/app/git/trees/{commit}:")) {
            return match r.tree(rest) {
                Some(sha) => Ok((
                    "200 OK",
                    None,
                    json!({ "sha": sha }).to_string().into_bytes(),
                )),
                None => not_found,
            };
        }
        if path == format!("/repos/acme/app/tarball/{commit}") {
            let to = format!("{codeload}/acme/app/legacy.tar.gz/{commit}");
            return Ok(("302 Found", Some(to), vec![]));
        }
        if path == format!("/acme/app/legacy.tar.gz/{commit}") {
            return Ok(("200 OK", None, r.archive()?));
        }
        not_found
    }

    fn spec(file: &str, context: Option<&str>) -> Result<SessionSpec> {
        Ok(serde_json::from_value(json!({
            "session_id": "ses_test",
            "number": 1,
            "title": "Test",
            "project": {
                "id": "prj_Test1",
                "slug": "app",
                "name": "App",
                "repo": { "owner": "acme", "name": "app" },
                "default_branch": "main",
                "settings": { "image_build": { "file": file, "context": context } }
            },
            "branch": "fctl/1-test",
            "base_branch": "main",
            "author": { "id": "usr_test", "name": "Ada" },
            "claude_session_id": "00000000-0000-0000-0000-000000000000",
            "resume": false,
            "secrets": {},
            "credentials": {
                "claude": { "id": "crd_test", "kind": "setup_token", "access_token": "x", "scopes": [] },
                "github": { "token": "gh-token", "expires_at": 0 }
            },
            "git_author": { "name": "fctl", "email": "fctl@example.com" },
            "co_authors": [],
            "web_url": "https://fctl.test/s/ses_test",
            "idle_timeout_secs": 600
        }))?)
    }

    struct Host {
        ctx: HostCtx,
        cli: FakeCli,
        _data: tempfile::TempDir,
        log: Mutex<Vec<String>>,
    }

    impl Host {
        fn new(github: &FakeGithub) -> Result<Self> {
            let cli = FakeCli::new(&[], &[])?;
            let data = tempfile::tempdir()?;
            let cfg = crate::HostConfig {
                data_dir: Some(data.path().to_owned()),
                github_api: github.url.clone(),
                ..Default::default()
            };
            let driver = Driver::Container(cli.driver(Flavor::Docker));
            Ok(Self {
                ctx: HostCtx::new(cfg, driver, "fctl_rt_test".into()),
                cli,
                _data: data,
                log: Mutex::new(Vec::new()),
            })
        }

        async fn image(&self, spec: &SessionSpec) -> Result<String> {
            let log = |line: String| {
                self.log
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(line)
            };
            image(&self.ctx, spec, &log).await
        }

        fn builds(&self) -> Vec<String> {
            let calls = self.cli.calls();
            calls
                .into_iter()
                .filter(|c| c.starts_with("build "))
                .collect()
        }

        fn has(&self, image: &str) -> bool {
            self.cli
                .dir
                .path()
                .join("have")
                .join(image.replace(['/', ':'], "_"))
                .exists()
        }

        fn log(&self) -> Vec<String> {
            self.log
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    #[tokio::test]
    async fn hosts_build_once_and_again_when_the_files_change() -> Result<()> {
        let github = FakeGithub::start(&[
            ("README.md", "# app\n"),
            ("docker/Dockerfile", "FROM debian\nCOPY . /app\n"),
            ("app/main.rs", "fn main() {}\n"),
        ])
        .await?;
        let host = Host::new(&github)?;
        let spec = spec("docker/Dockerfile", Some("app"))?;
        let tag = host.image(&spec).await?;
        assert!(tag.starts_with("fctl-build-prj_test1:"), "{tag}");
        assert!(host.has(&tag));
        let builds = host.builds();
        assert_eq!(builds.len(), 1, "{builds:?}");
        assert!(
            builds[0].starts_with(&format!("build --tag {tag} --file ")),
            "{builds:?}"
        );
        assert!(builds[0].contains("/src/docker/Dockerfile "), "{builds:?}");
        assert!(builds[0].ends_with("/src/app"), "{builds:?}");
        // It built in the context, from what the archive had there.
        let context = std::fs::read_to_string(host.cli.dir.path().join("context"))?;
        assert_eq!(context, ".\n./main.rs\n");
        let log = host.log();
        assert!(
            log.iter()
                .any(|l| l
                    == "Building the sandbox image from docker/Dockerfile in acme/app at 0000000"),
            "{log:?}"
        );
        assert!(log.iter().any(|l| l == "#5 [1/2] FROM debian"), "{log:?}");
        assert!(!log.iter().any(|l| l.contains("noise")), "{log:?}");
        // The token went to GitHub's API, not to where the archive came from.
        for (path, authorized) in github.requests() {
            assert_eq!(authorized, path.starts_with("/repos/"), "{path}");
        }

        // The next session starts from the build.
        assert_eq!(host.image(&spec).await?, tag);
        assert_eq!(host.builds().len(), 1);
        // So does one after a change it doesn't read.
        github.change(|r| {
            r.files.insert("README.md".into(), "# App\n".into());
        });
        assert_eq!(host.image(&spec).await?, tag);
        assert_eq!(host.builds().len(), 1);
        // A change in the context builds again, and the old build goes.
        github.change(|r| {
            r.files.insert("app/lib.rs".into(), "\n".into());
        });
        let rebuilt = host.image(&spec).await?;
        assert_ne!(rebuilt, tag);
        assert_eq!(host.builds().len(), 2);
        assert!(host.has(&rebuilt) && !host.has(&tag));
        // So does a change to the Dockerfile.
        github.change(|r| {
            r.files
                .insert("docker/Dockerfile".into(), "FROM debian:13\n".into());
        });
        let third = host.image(&spec).await?;
        assert_ne!(third, rebuilt);
        assert_eq!(host.builds().len(), 3);

        // While GitHub is down, sessions start from the last build.
        github.change(|r| r.down = true);
        assert_eq!(host.image(&spec).await?, third);
        assert!(
            host.log()
                .iter()
                .any(|l| l.starts_with("Could not check acme/app for changes"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn builds_that_fail_or_grow_too_big_start_no_session() -> Result<()> {
        let github = FakeGithub::start(&[("Dockerfile", "FROM debian\n")]).await?;
        let host = Host::new(&github)?;
        // A file the repository doesn't have.
        let e = host
            .image(&spec("Containerfile", None)?)
            .await
            .err()
            .map(|e| format!("{e:#}"));
        assert_eq!(
            e.as_deref(),
            Some("Containerfile isn't in acme/app on main (Settings → Sandbox → Image)")
        );
        // A build that fails says why.
        std::fs::write(host.cli.dir.path().join("fail_build"), "")?;
        let e = host
            .image(&spec("Dockerfile", None)?)
            .await
            .err()
            .map(|e| format!("{e:#}"));
        assert!(
            e.as_deref().is_some_and(
                |e| e.starts_with("Building the image failed (exit status: 1): #3 ERROR:")
            ),
            "{e:?}"
        );
        std::fs::remove_file(host.cli.dir.path().join("fail_build"))?;
        // An image larger than the host keeps is removed.
        std::fs::write(
            host.cli.dir.path().join("size"),
            format!("{}", 30_u64 << 30),
        )?;
        let e = host
            .image(&spec("Dockerfile", None)?)
            .await
            .err()
            .map(|e| format!("{e:#}"));
        assert_eq!(
            e.as_deref(),
            Some(
                "The image is 30 GiB, more than the 20 GiB this host keeps (max_build_mb in host.toml)"
            )
        );
        assert!(
            host.cli
                .calls()
                .iter()
                .any(|c| c.starts_with("image rm fctl-build-prj_test1:"))
        );
        // A dev container that names an image is that image, unbuilt.
        github.change(|r| {
            r.files.insert(
                ".devcontainer/devcontainer.json".into(),
                "{ // An image\n \"image\": \"mcr.microsoft.com/devcontainers/rust:1\", }".into(),
            );
        });
        let image = host
            .image(&spec(".devcontainer/devcontainer.json", None)?)
            .await?;
        assert_eq!(image, "mcr.microsoft.com/devcontainers/rust:1");
        Ok(())
    }
}
