//! Pulling images from their registries, as Docker does, without Docker:
//! the manifest for this machine's platform, the config, and the layers,
//! each checked against its digest.

use anyhow::{Context, Result, bail};
use fctl_client::HttpClient;
use fctl_domain::image::ImageRef;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

/// What a manifest may be, as registries name them: an index of the
/// platforms' manifests, or one platform's.
const ACCEPT: &str = "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json";

/// The largest manifest or config a registry may send.
const MAX_JSON: usize = 4 * 1024 * 1024;

/// The largest layer, and how long one may take.
pub const MAX_LAYER: u64 = 64 * 1024 * 1024 * 1024;
pub const LAYER_TIME: Duration = Duration::from_secs(60 * 60);

/// A blob: what it is, its digest and its size.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    #[serde(default)]
    pub media_type: String,
    pub digest: String,
    #[serde(default)]
    pub size: u64,
}

/// An image as its registry describes it for this machine.
#[derive(Clone, Debug)]
pub struct Resolved {
    /// The config's digest: the image's ID, as Docker's.
    pub id: String,
    pub config: Config,
    pub layers: Vec<Descriptor>,
}

/// What an image's config says about running it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Config {
    #[serde(rename = "User", default)]
    pub user: Option<String>,
    #[serde(rename = "Env", default)]
    pub env: Option<Vec<String>>,
    #[serde(rename = "WorkingDir", default)]
    pub working_dir: Option<String>,
    #[serde(rename = "Labels", default)]
    pub labels: Option<BTreeMap<String, String>>,
}

#[derive(Deserialize)]
struct ConfigFile {
    #[serde(default)]
    config: Config,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    #[serde(default)]
    media_type: Option<String>,
    #[serde(default)]
    manifests: Option<Vec<Platformed>>,
    #[serde(default)]
    config: Option<Descriptor>,
    #[serde(default)]
    layers: Option<Vec<Descriptor>>,
}

#[derive(Deserialize)]
struct Platformed {
    digest: String,
    #[serde(default)]
    platform: Option<Platform>,
}

#[derive(Deserialize)]
struct Platform {
    #[serde(default)]
    os: String,
    #[serde(default)]
    architecture: String,
}

/// An image's architecture, as registries name it: `arm64` for Rust's
/// `aarch64`.
pub fn architecture(arch: &str) -> &str {
    match arch {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => other,
    }
}

/// The registry and repository of `name` (`ghcr.io/acme/agent`), with
/// Docker Hub's API host for `docker.io`, and whether it speaks plain HTTP:
/// one on this machine does, as Docker allows.
fn endpoint(name: &str) -> Result<(String, &str)> {
    let (registry, repository) = name
        .split_once('/')
        .context("an image name without a registry")?;
    let host = registry.split(':').next().unwrap_or(registry);
    let scheme = if host == "localhost" || host.starts_with("127.") || host == "[::1]" {
        "http"
    } else {
        "https"
    };
    let registry = if registry == "docker.io" {
        "registry-1.docker.io"
    } else {
        registry
    };
    Ok((format!("{scheme}://{registry}/v2/{repository}"), repository))
}

/// The parts of a `WWW-Authenticate: Bearer realm="…",service="…"` header.
fn challenge(header: &str) -> Option<BTreeMap<String, String>> {
    let rest = header.trim().strip_prefix("Bearer ")?;
    let mut out = BTreeMap::new();
    let mut s = rest;
    while !s.is_empty() {
        let (key, after) = s.split_once('=')?;
        let key = key
            .trim()
            .trim_start_matches(',')
            .trim()
            .to_ascii_lowercase();
        let after = after.trim_start();
        let (value, rest) = match after.strip_prefix('"') {
            Some(quoted) => {
                let end = quoted.find('"')?;
                (&quoted[..end], &quoted[end + 1..])
            }
            None => match after.find(',') {
                Some(i) => (&after[..i], &after[i..]),
                None => (after, ""),
            },
        };
        out.insert(key, value.to_owned());
        s = rest.trim_start_matches(',').trim_start();
    }
    Some(out)
}

/// The manifest of `os`/`arch` in an index.
fn pick(index: &[Platformed], os: &str, arch: &str) -> Option<String> {
    index
        .iter()
        .find(|m| {
            m.platform
                .as_ref()
                .is_some_and(|p| p.os == os && p.architecture == arch)
        })
        .map(|m| m.digest.clone())
}

/// `sha256:…` of `bytes`.
pub fn digest_of(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

/// `sha256:…` of the file at `path`.
pub fn file_digest(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

/// The credentials `docker login` keeps for `registry` in
/// `~/.docker/config.json` (`auths`, not credential helpers): the value
/// of a basic `Authorization`.
fn docker_login(registry: &str) -> Option<String> {
    let dir = std::env::var_os("DOCKER_CONFIG")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".docker")))?;
    let text = std::fs::read_to_string(dir.join("config.json")).ok()?;
    login_in(&text, registry)
}

fn login_in(config: &str, registry: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(config).ok()?;
    let auths = v.get("auths")?.as_object()?;
    let names: &[&str] = if registry == "docker.io" {
        &[
            "https://index.docker.io/v1/",
            "index.docker.io",
            "docker.io",
        ]
    } else {
        &[registry]
    };
    let auth = names.iter().find_map(|n| {
        auths
            .get(*n)
            .or_else(|| auths.get(&format!("https://{n}")))?
            .get("auth")?
            .as_str()
    })?;
    Some(format!("Basic {auth}"))
}

/// A registry client for one pull.
pub struct Registry {
    http: HttpClient,
    /// Tokens for the repositories this pull read, by repository.
    tokens: Mutex<BTreeMap<String, String>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub fn new() -> Self {
        Self {
            http: HttpClient::new(Duration::from_secs(60)),
            tokens: Mutex::new(BTreeMap::new()),
        }
    }

    fn token(&self, repository: &str) -> Option<String> {
        self.tokens.lock().ok()?.get(repository).cloned()
    }

    /// A GET of `url` for `name`, asking the registry's token service for a
    /// token when it wants one, and following redirects (Docker Hub's blobs
    /// are on a CDN), where the token doesn't go.
    async fn get(&self, name: &str, url: &str, accept: &str) -> Result<(http::HeaderMap, Vec<u8>)> {
        let (_, repository) = endpoint(name)?;
        let registry = name.split('/').next().unwrap_or_default();
        let origin = |u: &str| url::Url::parse(u).ok().map(|u| u.origin());
        let mut current = url.to_owned();
        let mut signed_in = false;
        for _ in 0..6 {
            let token = self
                .token(repository)
                .filter(|_| origin(&current) == origin(url));
            let (status, headers, body) = self
                .http
                .get_with(&current, token.as_deref(), &[("accept", accept)])
                .await
                .with_context(|| format!("reaching {registry}"))?;
            if status.as_u16() == 401 && !signed_in && origin(&current) == origin(url) {
                let header = headers
                    .get(http::header::WWW_AUTHENTICATE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default();
                let token = self.sign_in(registry, repository, header).await?;
                if let Ok(mut tokens) = self.tokens.lock() {
                    tokens.insert(repository.to_owned(), token);
                }
                signed_in = true;
                continue;
            }
            if status.is_redirection() {
                let location = headers
                    .get(http::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .with_context(|| format!("{registry} redirected nowhere"))?;
                current = url::Url::parse(&current)
                    .and_then(|base| base.join(location))
                    .context("the registry's redirect")?
                    .to_string();
                continue;
            }
            if !status.is_success() {
                let why = String::from_utf8_lossy(&body)
                    .chars()
                    .take(300)
                    .collect::<String>();
                bail!("{registry} answered {status}: {}", why.trim());
            }
            if body.len() > MAX_JSON {
                bail!("{registry} sent more than {} MiB", MAX_JSON / 1024 / 1024);
            }
            return Ok((headers, body.to_vec()));
        }
        bail!("{registry} redirected too often, or refused the token it gave")
    }

    /// A token for pulling `repository`, from the service the registry's
    /// challenge names: anonymous, or with the credentials of `docker login`.
    async fn sign_in(&self, registry: &str, repository: &str, header: &str) -> Result<String> {
        let c = challenge(header)
            .with_context(|| format!("{registry} asks to sign in, in a way fctl doesn't know"))?;
        let realm = c
            .get("realm")
            .context("no realm in the registry's challenge")?;
        let mut url = url::Url::parse(realm).context("the registry's token service")?;
        {
            let mut q = url.query_pairs_mut();
            if let Some(service) = c.get("service") {
                q.append_pair("service", service);
            }
            q.append_pair("scope", &format!("repository:{repository}:pull"));
        }
        let login = docker_login(registry);
        let mut headers = vec![("accept", "application/json")];
        if let Some(auth) = login.as_deref() {
            headers.push(("authorization", auth));
        }
        let (status, _, body) = self
            .http
            .get_with(url.as_str(), None, &headers)
            .await
            .with_context(|| format!("signing in to {registry}"))?;
        if !status.is_success() {
            bail!(
                "{registry} gave no token to pull {repository} ({status}){}",
                if login.is_none() {
                    ": a private image needs `docker login` on this machine"
                } else {
                    ""
                }
            );
        }
        let v: serde_json::Value = serde_json::from_slice(&body).context("the registry's token")?;
        v.get("token")
            .or_else(|| v.get("access_token"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .context("no token in the registry's answer")
    }

    /// The image `reference` names, for `linux/<arch>`.
    pub async fn resolve(&self, reference: &str, arch: &str) -> Result<Resolved> {
        let r = ImageRef::parse(reference)
            .with_context(|| format!("{reference} isn't an image reference"))?;
        let (base, _) = endpoint(&r.name)?;
        let wanted = r
            .digest
            .clone()
            .unwrap_or_else(|| r.tag.clone().unwrap_or_else(|| "latest".into()));
        let mut manifest_ref = wanted.clone();
        // An index points at a manifest, which points at no other.
        for _ in 0..2 {
            let url = format!("{base}/manifests/{manifest_ref}");
            let (_, body) = self.get(&r.name, &url, ACCEPT).await?;
            if manifest_ref.starts_with("sha256:") && digest_of(&body) != manifest_ref {
                bail!("the manifest of {reference} doesn't match its digest");
            }
            let m: Manifest = serde_json::from_slice(&body)
                .with_context(|| format!("reading the manifest of {reference}"))?;
            if let Some(index) = &m.manifests {
                manifest_ref = pick(index, "linux", architecture(arch)).with_context(|| {
                    format!("{reference} has no image for linux/{}", architecture(arch))
                })?;
                continue;
            }
            let (Some(config), Some(layers)) = (m.config, m.layers) else {
                bail!(
                    "{reference}'s manifest ({}) isn't one fctl reads",
                    m.media_type.unwrap_or_default()
                );
            };
            let url = format!("{base}/blobs/{}", config.digest);
            let (_, body) = self.get(&r.name, &url, "*/*").await?;
            if digest_of(&body) != config.digest {
                bail!("the config of {reference} doesn't match its digest");
            }
            let file: ConfigFile = serde_json::from_slice(&body)
                .with_context(|| format!("reading the config of {reference}"))?;
            return Ok(Resolved {
                id: config.digest,
                config: file.config,
                layers,
            });
        }
        bail!("{reference}'s index points at another index")
    }

    /// Downloads the blob `layer` of the image `reference` to `to`, and
    /// checks it against its digest.
    pub async fn blob(&self, reference: &str, layer: &Descriptor, to: &Path) -> Result<()> {
        let r = ImageRef::parse(reference)
            .with_context(|| format!("{reference} isn't an image reference"))?;
        let (base, repository) = endpoint(&r.name)?;
        let url = format!("{base}/blobs/{}", layer.digest);
        // A layer of an image that wants no token yet, after its manifest
        // did, gets one: tokens are per repository.
        let token = self.token(repository);
        let http = HttpClient::new(LAYER_TIME);
        let size = http
            .download_to(&url, token.as_deref(), &[], to, MAX_LAYER, LAYER_TIME)
            .await
            .with_context(|| format!("downloading {}", layer.digest))?;
        if layer.size > 0 && size != layer.size {
            bail!(
                "{} is {size} bytes rather than the {} its manifest says",
                layer.digest,
                layer.size
            );
        }
        let path = to.to_owned();
        let digest = tokio::task::spawn_blocking(move || file_digest(&path)).await??;
        if digest != layer.digest {
            bail!("{} doesn't match its digest", layer.digest);
        }
        Ok(())
    }
}

/// The value of a basic `Authorization` for `user` and `password`.
#[cfg(test)]
fn basic(user: &str, password: &str) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
    format!("Basic {encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registries_have_their_api_hosts() -> Result<()> {
        assert_eq!(
            endpoint("ghcr.io/hyperstructure/sandbox")?,
            (
                "https://ghcr.io/v2/hyperstructure/sandbox".to_owned(),
                "hyperstructure/sandbox"
            )
        );
        assert_eq!(
            endpoint("docker.io/library/debian")?.0,
            "https://registry-1.docker.io/v2/library/debian"
        );
        assert_eq!(
            endpoint("localhost:5000/test/sandbox")?.0,
            "http://localhost:5000/v2/test/sandbox"
        );
        assert_eq!(
            endpoint("127.0.0.1:5000/a/b")?.0,
            "http://127.0.0.1:5000/v2/a/b"
        );
        assert_eq!(architecture("aarch64"), "arm64");
        assert_eq!(architecture("x86_64"), "amd64");
        Ok(())
    }

    #[test]
    fn reads_the_registrys_challenge() {
        let c = challenge(
            r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:hyperstructure/sandbox:pull""#,
        );
        assert_eq!(
            c.as_ref().and_then(|c| c.get("realm")).map(String::as_str),
            Some("https://ghcr.io/token")
        );
        assert_eq!(
            c.as_ref()
                .and_then(|c| c.get("service"))
                .map(String::as_str),
            Some("ghcr.io")
        );
        let unquoted = challenge("Bearer realm=https://auth.example/token, service=reg");
        assert_eq!(
            unquoted.and_then(|c| c.get("service").cloned()).as_deref(),
            Some("reg")
        );
        assert_eq!(challenge(r#"Basic realm="x""#), None);
    }

    #[test]
    fn picks_this_machines_platform() -> Result<()> {
        let index: Manifest = serde_json::from_str(
            r#"{"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[
                {"digest":"sha256:arm","platform":{"os":"linux","architecture":"arm64"}},
                {"digest":"sha256:amd","platform":{"os":"linux","architecture":"amd64"}},
                {"digest":"sha256:att","platform":{"os":"unknown","architecture":"unknown"}}]}"#,
        )?;
        let manifests = index.manifests.unwrap_or_default();
        assert_eq!(
            pick(&manifests, "linux", "arm64").as_deref(),
            Some("sha256:arm")
        );
        assert_eq!(
            pick(&manifests, "linux", "amd64").as_deref(),
            Some("sha256:amd")
        );
        assert_eq!(pick(&manifests, "linux", "riscv64"), None);
        Ok(())
    }

    #[test]
    fn uses_the_credentials_of_docker_login() {
        use base64::Engine;
        let config = format!(
            r#"{{"auths":{{"ghcr.io":{{"auth":"{}"}},"https://index.docker.io/v1/":{{"auth":"aHViOnB3"}}}},"credsStore":"desktop"}}"#,
            base64::engine::general_purpose::STANDARD.encode("ada:secret")
        );
        assert_eq!(login_in(&config, "ghcr.io"), Some(basic("ada", "secret")));
        assert_eq!(
            login_in(&config, "docker.io").as_deref(),
            Some("Basic aHViOnB3")
        );
        assert_eq!(login_in(&config, "quay.io"), None);
        assert_eq!(login_in("not json", "ghcr.io"), None);
    }

    #[test]
    fn reads_what_an_image_says_about_running_it() -> Result<()> {
        let file: ConfigFile = serde_json::from_str(
            r#"{"architecture":"arm64","config":{"User":"agent","Env":["PATH=/usr/bin"],"WorkingDir":"/home/agent","Labels":{"org.opencontainers.image.revision":"f392b5c"}}}"#,
        )?;
        assert_eq!(file.config.user.as_deref(), Some("agent"));
        assert_eq!(file.config.env, Some(vec!["PATH=/usr/bin".to_owned()]));
        assert_eq!(
            file.config
                .labels
                .and_then(|l| l.get("org.opencontainers.image.revision").cloned())
                .as_deref(),
            Some("f392b5c")
        );
        let bare: ConfigFile = serde_json::from_str(r#"{"config":{"Env":null}}"#)?;
        assert_eq!(bare.config, Config::default());
        Ok(())
    }
}
