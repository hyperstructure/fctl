//! A bucket of the organization's own that the host keeps its sessions'
//! snapshots in, in place of the control plane (`[snapshots]` in
//! host.toml, docs/architecture.md#session-snapshots): S3's API, signed with
//! SigV4 (`fctl_domain::sigv4`), with keys from host.toml, the `AWS_*`
//! variables, a web identity (EKS), a container's credentials (EKS Pod
//! Identity, ECS) or the machine's instance role (EC2), as AWS's own tools
//! look for them. S3-compatible services (R2, MinIO, Google Cloud
//! Storage's XML API) work with an `endpoint`.

use crate::config::SnapshotsConfig;
use fctl_client::http::HttpClient;
use fctl_domain::sigv4::{self, Credentials};
use fctl_domain::snapshots::Bucket;
use fctl_proto::runner::SnapshotStore;
use http::Method;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Objects up to this size go in one PUT; larger ones in parts of
/// [`PART_BYTES`].
const SINGLE_PUT_MAX: u64 = 64 * 1024 * 1024;
const PART_BYTES: usize = 16 * 1024 * 1024;
/// How long a role's credentials are used before they are asked for
/// again: every source makes new ones at least 5 minutes before the old
/// expire.
const CREDENTIALS_FOR: Duration = Duration::from_secs(5 * 60);
/// The largest snapshot a bucket keeps unless host.toml says otherwise.
pub const DEFAULT_MAX_MB: u32 = 2048;
/// How many times a request to the bucket is tried in all while it fails
/// in a way that may pass ([`try_again`]): S3 asks clients to, and a 2 GiB
/// snapshot takes some 130 requests.
const TRIES: u32 = 4;
/// How long the second try waits; each one after it three times as long
/// (1, 3 and 9 seconds).
const FIRST_WAIT: Duration = Duration::from_secs(1);

/// Where requests go: the host (and, for path-style addressing, the
/// bucket's name first in the path).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Endpoint {
    scheme: String,
    host: String,
    path_style: bool,
}

/// Where the keys come from.
#[derive(Clone, Debug)]
enum Source {
    /// host.toml's `access_key_id`, and its secret from a file.
    Static { id: String, secret_file: PathBuf },
    /// The machine's: the `AWS_*` variables, a web identity, a container's
    /// credentials, or the instance's role.
    Machine,
}

pub struct Store {
    pub bucket: Bucket,
    endpoint: Endpoint,
    region: String,
    source: Source,
    /// The largest snapshot it keeps, in bytes.
    pub max_bytes: u64,
    http: HttpClient,
    cached: Mutex<Option<(Credentials, Instant)>>,
    /// What the host says of it in its hello and heartbeats.
    state: Mutex<SnapshotStore>,
    /// How long a request waits before its second try ([`FIRST_WAIT`]).
    first_wait: Duration,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("bucket", &self.bucket.url())
            .field("region", &self.region)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// The bucket `cfg` names, if it names one. Fails on a URL it can't
    /// read: snapshots meant for an organization's own bucket never go to
    /// the control plane by mistake.
    pub async fn open(cfg: &SnapshotsConfig) -> anyhow::Result<Option<Self>> {
        let Some(url) = &cfg.url else {
            return Ok(None);
        };
        let bucket = Bucket::parse(url).map_err(|e| anyhow::anyhow!("[snapshots] url: {e}"))?;
        let http = HttpClient::new(Duration::from_secs(5 * 60));
        let region = match (&cfg.region, &cfg.endpoint) {
            (Some(r), _) => r.clone(),
            (None, endpoint) => match env_region() {
                Some(r) => r,
                // On EC2, the machine's own; elsewhere, S3's first.
                None if endpoint.is_none() => {
                    imds_region().await.unwrap_or_else(|| "us-east-1".into())
                }
                None => "us-east-1".into(),
            },
        };
        let endpoint = endpoint(cfg.endpoint.as_deref(), &bucket.name, &region)?;
        let source = match (&cfg.access_key_id, &cfg.secret_access_key_file) {
            (Some(id), Some(file)) => Source::Static {
                id: id.clone(),
                secret_file: file.clone(),
            },
            (None, None) => Source::Machine,
            _ => anyhow::bail!(
                "[snapshots] takes access_key_id and secret_access_key_file together, or neither"
            ),
        };
        let max_bytes = u64::from(cfg.max_mb.unwrap_or(DEFAULT_MAX_MB)) * 1024 * 1024;
        Ok(Some(Self {
            state: Mutex::new(SnapshotStore {
                url: bucket.url(),
                error: None,
                failed_at: None,
            }),
            bucket,
            endpoint,
            region,
            source,
            max_bytes,
            http,
            cached: Mutex::new(None),
            first_wait: FIRST_WAIT,
        }))
    }

    /// What the host says of it: its URL, and why it last failed while it
    /// does.
    pub fn report(&self) -> SnapshotStore {
        self.state
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| SnapshotStore {
                url: self.bucket.url(),
                error: None,
                failed_at: None,
            })
    }

    fn note<T>(&self, result: Result<T, String>) -> Result<T, String> {
        if let Ok(mut s) = self.state.lock() {
            match &result {
                Ok(_) => {
                    s.error = None;
                    s.failed_at = None;
                }
                Err(e) => {
                    s.error = Some(e.clone());
                    s.failed_at = Some(crate::expiry::now_ms());
                }
            }
        }
        result
    }

    /// Puts the file at `path`, whose SHA-256 is `sha256` (hex), at `key`.
    pub async fn put(&self, key: &str, path: &Path, sha256: &str) -> Result<(), String> {
        let result = self.put_inner(key, path, sha256).await;
        self.note(result)
    }

    async fn put_inner(&self, key: &str, path: &Path, sha256: &str) -> Result<(), String> {
        let size = std::fs::metadata(path)
            .map_err(|e| format!("reading the snapshot: {e}"))?
            .len();
        if size <= SINGLE_PUT_MAX {
            let body = tokio::fs::read(path)
                .await
                .map_err(|e| format!("reading the snapshot: {e}"))?;
            let (status, _, answer) = self
                .request(Method::PUT, key, &[], Some(body.into()), sha256)
                .await?;
            return expect_ok(status, &answer, "putting it in the bucket");
        }
        // In parts, the upload let go of should one fail.
        let (status, _, answer) = self
            .request(
                Method::POST,
                key,
                &[("uploads", "")],
                Some(bytes::Bytes::new()),
                sigv4::EMPTY_SHA256,
            )
            .await?;
        expect_ok(status, &answer, "starting its upload")?;
        let upload = xml_text(&answer, "UploadId")
            .ok_or_else(|| "the bucket started no upload".to_owned())?;
        let parts = self.put_parts(key, path, &upload).await;
        let done = match parts {
            Ok(etags) => {
                let body = complete_body(&etags);
                let hash = sigv4::sha256_hex(body.as_bytes());
                let completed = self
                    .request(
                        Method::POST,
                        key,
                        &[("uploadId", upload.as_str())],
                        Some(body.into_bytes().into()),
                        &hash,
                    )
                    .await;
                match completed {
                    // S3 can answer a failed completion with 200 and an error.
                    Ok((_, _, answer)) if answer.windows(7).any(|w| w == b"<Error>") => {
                        Err(format!(
                            "finishing its upload failed: {}",
                            xml_text(&answer, "Message").unwrap_or_default()
                        ))
                    }
                    Ok((status, _, answer)) => expect_ok(status, &answer, "finishing its upload"),
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        };
        // Its parts would stay, and cost, until a lifecycle rule took them.
        if done.is_err() {
            let _ = self
                .request(
                    Method::DELETE,
                    key,
                    &[("uploadId", upload.as_str())],
                    None,
                    sigv4::EMPTY_SHA256,
                )
                .await;
        }
        done
    }

    async fn put_parts(&self, key: &str, path: &Path, upload: &str) -> Result<Vec<String>, String> {
        use tokio::io::AsyncReadExt;
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|e| format!("reading the snapshot: {e}"))?;
        let mut etags = Vec::new();
        let mut buf = vec![0u8; PART_BYTES];
        loop {
            let mut len = 0;
            while len < buf.len() {
                match file.read(buf.get_mut(len..).unwrap_or_default()).await {
                    Ok(0) => break,
                    Ok(n) => len += n,
                    Err(e) => return Err(format!("reading the snapshot: {e}")),
                }
            }
            if len == 0 {
                break;
            }
            let part = bytes::Bytes::copy_from_slice(buf.get(..len).unwrap_or_default());
            let hash = hex::encode(Sha256::digest(&part));
            let number = (etags.len() + 1).to_string();
            let (status, headers, answer) = self
                .request(
                    Method::PUT,
                    key,
                    &[("partNumber", number.as_str()), ("uploadId", upload)],
                    Some(part),
                    &hash,
                )
                .await?;
            expect_ok(status, &answer, "putting a part of it")?;
            let etag = headers
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| "the bucket gave a part no ETag".to_owned())?;
            etags.push(etag.to_owned());
        }
        Ok(etags)
    }

    /// Gets the object at `key` into the file at `path`, at most
    /// `max_bytes`: its size.
    pub async fn get(&self, key: &str, path: &Path, max_bytes: u64) -> Result<u64, String> {
        let result = self.get_inner(key, path, max_bytes).await;
        // An object the bucket's lifecycle let go of says nothing of the
        // bucket itself.
        match &result {
            Err(e) if e.contains("NoSuchKey") || e.contains("HTTP 404") => result,
            _ => self.note(result),
        }
    }

    async fn get_inner(&self, key: &str, path: &Path, max_bytes: u64) -> Result<u64, String> {
        let mut wait = self.first_wait;
        let mut tries = 1;
        loop {
            // Signed again for each try, as S3 takes a signature for 15
            // minutes.
            let (url, signed) = self
                .sign(&Method::GET, key, &[], sigv4::EMPTY_SHA256)
                .await?;
            let headers: Vec<(&str, &str)> = signed
                .iter()
                .map(|(n, v)| (n.as_str(), v.as_str()))
                .collect();
            let got = self
                .http
                .download_to(
                    &url,
                    None,
                    &headers,
                    path,
                    max_bytes,
                    Duration::from_secs(30 * 60),
                )
                .await;
            match got {
                Ok(size) => return Ok(size),
                // A download that ran out of its time would again.
                Err(e)
                    if tries < TRIES
                        && e.is_transient()
                        && !matches!(e, fctl_client::Error::Timeout(_)) =>
                {
                    tracing::debug!("getting {key} from the bucket, again in {wait:?}: {e}");
                }
                Err(e) => return Err(format!("getting it from the bucket failed: {e}")),
            }
            tokio::time::sleep(wait).await;
            wait *= 3;
            tries += 1;
        }
    }

    /// Deletes the object at `key`.
    pub async fn delete(&self, key: &str) -> Result<(), String> {
        let (status, _, answer) = self
            .request(Method::DELETE, key, &[], None, sigv4::EMPTY_SHA256)
            .await?;
        expect_ok(status, &answer, "deleting it")
    }

    /// Sends a request to `key`, again while it fails in a way that may
    /// pass ([`try_again`]), signed anew and with its body each time: the
    /// last answer, whatever its status.
    async fn request(
        &self,
        method: Method,
        key: &str,
        query: &[(&str, &str)],
        body: Option<bytes::Bytes>,
        sha256: &str,
    ) -> Result<(http::StatusCode, http::HeaderMap, bytes::Bytes), String> {
        let mut wait = self.first_wait;
        let mut tries = 1;
        loop {
            let (url, signed) = self.sign(&method, key, query, sha256).await?;
            let headers: Vec<(&str, &str)> = signed
                .iter()
                .map(|(n, v)| (n.as_str(), v.as_str()))
                .collect();
            let sent = self
                .http
                .send_with(method.clone(), &url, &headers, body.clone())
                .await;
            let again = match &sent {
                Ok((status, _, answer)) => try_again(*status, answer),
                // A request that ran out of its 5 minutes isn't tried again:
                // four of them would outlast the session's wait for its
                // upload (`snapshots::UPLOAD_LIMIT`).
                Err(e) => e.is_transient() && !matches!(e, fctl_client::Error::Timeout(_)),
            };
            if !again || tries >= TRIES {
                return sent.map_err(|e| format!("the bucket didn't answer: {e}"));
            }
            tracing::debug!("{method} {key} in the bucket, again in {wait:?}");
            tokio::time::sleep(wait).await;
            wait *= 3;
            tries += 1;
        }
    }

    /// The URL of `key` with `query`, and the headers that sign a request
    /// to it.
    async fn sign(
        &self,
        method: &Method,
        key: &str,
        query: &[(&str, &str)],
        sha256: &str,
    ) -> Result<(String, Vec<(String, String)>), String> {
        let creds = self.credentials().await?;
        let path = if self.endpoint.path_style {
            format!("/{}/{key}", self.bucket.name)
        } else {
            format!("/{key}")
        };
        let signed = sigv4::sign(
            &sigv4::Request {
                method: method.as_str(),
                host: &self.endpoint.host,
                path: &path,
                query,
                headers: &[],
                payload_sha256: sha256,
            },
            &creds,
            &self.region,
            "s3",
            crate::expiry::now_ms(),
        );
        let query = query
            .iter()
            .map(|(k, v)| {
                if v.is_empty() {
                    sigv4::encode(k, false)
                } else {
                    format!("{}={}", sigv4::encode(k, false), sigv4::encode(v, false))
                }
            })
            .collect::<Vec<_>>()
            .join("&");
        let url = format!(
            "{}://{}{}{}{query}",
            self.endpoint.scheme,
            self.endpoint.host,
            sigv4::encode(&path, true),
            if query.is_empty() { "" } else { "?" }
        );
        Ok((url, signed))
    }

    async fn credentials(&self) -> Result<Credentials, String> {
        if let Ok(cached) = self.cached.lock()
            && let Some((creds, at)) = cached.as_ref()
            && at.elapsed() < CREDENTIALS_FOR
        {
            return Ok(creds.clone());
        }
        let creds = match &self.source {
            Source::Static { id, secret_file } => Credentials {
                access_key_id: id.clone(),
                secret_access_key: std::fs::read_to_string(secret_file)
                    .map_err(|e| format!("reading {}: {e}", secret_file.display()))?
                    .trim()
                    .to_owned(),
                session_token: None,
            },
            Source::Machine => machine_credentials(&self.http, &self.region).await?,
        };
        if let Ok(mut cached) = self.cached.lock() {
            *cached = Some((creds.clone(), Instant::now()));
        }
        Ok(creds)
    }
}

/// Where requests to `bucket` go: AWS's regional endpoint, by the bucket's
/// name as a host where it can be one; or `endpoint`, by path.
fn endpoint(endpoint: Option<&str>, bucket: &str, region: &str) -> anyhow::Result<Endpoint> {
    match endpoint {
        Some(e) => {
            let url =
                url::Url::parse(e).map_err(|err| anyhow::anyhow!("[snapshots] endpoint: {err}"))?;
            let host = url
                .host_str()
                .ok_or_else(|| anyhow::anyhow!("[snapshots] endpoint has no host"))?;
            Ok(Endpoint {
                scheme: url.scheme().to_owned(),
                host: match url.port() {
                    Some(p) => format!("{host}:{p}"),
                    None => host.to_owned(),
                },
                path_style: true,
            })
        }
        // A name with dots doesn't fit S3's certificate as a host.
        None if bucket.contains('.') => Ok(Endpoint {
            scheme: "https".into(),
            host: format!("s3.{region}.amazonaws.com"),
            path_style: true,
        }),
        None => Ok(Endpoint {
            scheme: "https".into(),
            host: format!("{bucket}.s3.{region}.amazonaws.com"),
            path_style: false,
        }),
    }
}

fn env_region() -> Option<String> {
    ["AWS_REGION", "AWS_DEFAULT_REGION"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|r| !r.trim().is_empty()))
}

const IMDS: &str = "http://169.254.169.254/latest";

/// A token for EC2's instance metadata (IMDSv2), on EC2.
async fn imds_token(http: &HttpClient) -> Option<String> {
    let (status, _, body) = http
        .send_with(
            Method::PUT,
            &format!("{IMDS}/api/token"),
            &[("x-aws-ec2-metadata-token-ttl-seconds", "300")],
            Some(bytes::Bytes::new()),
        )
        .await
        .ok()?;
    status
        .is_success()
        .then(|| String::from_utf8_lossy(&body).trim().to_owned())
}

async fn imds_get(http: &HttpClient, token: &str, path: &str) -> Option<String> {
    let (status, _, body) = http
        .send_with(
            Method::GET,
            &format!("{IMDS}/meta-data/{path}"),
            &[("x-aws-ec2-metadata-token", token)],
            None,
        )
        .await
        .ok()?;
    status
        .is_success()
        .then(|| String::from_utf8_lossy(&body).trim().to_owned())
}

/// The machine's region, on EC2.
async fn imds_region() -> Option<String> {
    let quick = HttpClient::new(Duration::from_secs(2));
    let token = imds_token(&quick).await?;
    imds_get(&quick, &token, "placement/region").await
}

/// The machine's keys, where AWS's own tools look for them: the `AWS_*`
/// variables, a web identity (EKS), a container's credentials (EKS Pod
/// Identity, ECS), the instance's role (EC2).
async fn machine_credentials(http: &HttpClient, region: &str) -> Result<Credentials, String> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
    if let (Some(id), Some(secret)) = (var("AWS_ACCESS_KEY_ID"), var("AWS_SECRET_ACCESS_KEY")) {
        return Ok(Credentials {
            access_key_id: id,
            secret_access_key: secret,
            session_token: var("AWS_SESSION_TOKEN"),
        });
    }
    if let (Some(role), Some(file)) = (var("AWS_ROLE_ARN"), var("AWS_WEB_IDENTITY_TOKEN_FILE")) {
        let token = std::fs::read_to_string(&file).map_err(|e| format!("reading {file}: {e}"))?;
        let sts = format!("https://sts.{region}.amazonaws.com/");
        // In the body, not the URL, which errors and logs may show: the
        // token is worth the role's keys until it expires.
        let form = [
            ("Action", "AssumeRoleWithWebIdentity"),
            ("DurationSeconds", "3600"),
            ("RoleArn", role.as_str()),
            ("RoleSessionName", "fctl-host"),
            ("Version", "2011-06-15"),
            ("WebIdentityToken", token.trim()),
        ]
        .iter()
        .map(|(k, v)| format!("{k}={}", sigv4::encode(v, false)))
        .collect::<Vec<_>>()
        .join("&");
        let (status, _, body) = http
            .send_with(
                Method::POST,
                &sts,
                // STS answers in JSON to the client's usual Accept.
                &[
                    ("content-type", "application/x-www-form-urlencoded"),
                    ("accept", "text/xml"),
                ],
                Some(form.into_bytes().into()),
            )
            .await
            .map_err(|e| format!("asking STS for the role's keys: {e}"))?;
        expect_ok(status, &body, "asking STS for the role's keys")?;
        return keys_from_xml(&body).ok_or_else(|| "STS answered with no keys".into());
    }
    let container = var("AWS_CONTAINER_CREDENTIALS_FULL_URI").or_else(|| {
        var("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI").map(|p| format!("http://169.254.170.2{p}"))
    });
    if let Some(uri) = container {
        let auth = match (
            var("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE"),
            var("AWS_CONTAINER_AUTHORIZATION_TOKEN"),
        ) {
            (Some(file), _) => Some(
                std::fs::read_to_string(&file)
                    .map_err(|e| format!("reading {file}: {e}"))?
                    .trim()
                    .to_owned(),
            ),
            (None, token) => token,
        };
        let headers: Vec<(&str, &str)> =
            auth.iter().map(|a| ("authorization", a.as_str())).collect();
        let (status, _, body) = http
            .send_with(Method::GET, &uri, &headers, None)
            .await
            .map_err(|e| format!("asking the container's agent for keys: {e}"))?;
        expect_ok(status, &body, "asking the container's agent for keys")?;
        return keys_from_json(&body).ok_or_else(|| "the container's agent gave no keys".into());
    }
    let quick = HttpClient::new(Duration::from_secs(5));
    let token = imds_token(&quick).await.ok_or_else(|| {
        "no keys: no AWS_* variables, web identity, container credentials or instance role"
            .to_owned()
    })?;
    let role = imds_get(&quick, &token, "iam/security-credentials/")
        .await
        .and_then(|r| r.lines().next().map(str::to_owned))
        .ok_or_else(|| "the instance has no role".to_owned())?;
    let keys = imds_get(&quick, &token, &format!("iam/security-credentials/{role}"))
        .await
        .ok_or_else(|| "the instance's role gave no keys".to_owned())?;
    keys_from_json(keys.as_bytes()).ok_or_else(|| "the instance's role gave no keys".into())
}

/// Keys in the JSON that EC2's metadata and containers' agents answer
/// with: `AccessKeyId`, `SecretAccessKey`, `Token`.
fn keys_from_json(body: &[u8]) -> Option<Credentials> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    Some(Credentials {
        access_key_id: v.get("AccessKeyId")?.as_str()?.to_owned(),
        secret_access_key: v.get("SecretAccessKey")?.as_str()?.to_owned(),
        session_token: v.get("Token").and_then(|t| t.as_str()).map(str::to_owned),
    })
}

/// Keys in STS's XML answer.
fn keys_from_xml(body: &[u8]) -> Option<Credentials> {
    Some(Credentials {
        access_key_id: xml_text(body, "AccessKeyId")?,
        secret_access_key: xml_text(body, "SecretAccessKey")?,
        session_token: xml_text(body, "SessionToken"),
    })
}

/// The text of the first `<tag>` in an XML answer.
fn xml_text(body: &[u8], tag: &str) -> Option<String> {
    let text = std::str::from_utf8(body).ok()?;
    let start = text.find(&format!("<{tag}>"))? + tag.len() + 2;
    let end = start + text.get(start..)?.find(&format!("</{tag}>"))?;
    Some(text.get(start..end)?.to_owned())
}

fn complete_body(etags: &[String]) -> String {
    let parts: String = etags
        .iter()
        .enumerate()
        .map(|(i, etag)| {
            format!(
                "<Part><PartNumber>{}</PartNumber><ETag>{etag}</ETag></Part>",
                i + 1
            )
        })
        .collect();
    format!("<CompleteMultipartUpload>{parts}</CompleteMultipartUpload>")
}

/// Whether an answer of the bucket says to try again: S3 asks for it with
/// a 5xx or 429 (`SlowDown`, `InternalError`), with a 400 `RequestTimeout`
/// when a body came too slowly, and with an `InternalError` in the 200 that
/// answers an upload's completion.
fn try_again(status: http::StatusCode, answer: &[u8]) -> bool {
    if status.is_server_error() || status == http::StatusCode::TOO_MANY_REQUESTS {
        return true;
    }
    answer.windows(7).any(|w| w == b"<Error>")
        && matches!(
            xml_text(answer, "Code").as_deref(),
            Some("RequestTimeout" | "InternalError" | "SlowDown" | "ServiceUnavailable")
        )
}

fn expect_ok(status: http::StatusCode, body: &[u8], doing: &str) -> Result<(), String> {
    if status.is_success() {
        return Ok(());
    }
    let code = xml_text(body, "Code").unwrap_or_else(|| status.to_string());
    Err(format!("{doing} failed: {code}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for S3: keeps objects by path, takes uploads in parts,
    /// and refuses requests that aren't signed with `AKID`'s key. It drops
    /// the connection that would complete an upload to a key with `drop`
    /// in it, and asks a request to a key with `slow` in it to slow down
    /// the first time it comes. Its URL, and the requests it had (`METHOD
    /// target`).
    async fn stand_in() -> anyhow::Result<(String, std::sync::Arc<Mutex<Vec<String>>>)> {
        use std::collections::HashMap;
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let objects: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
        let parts: Arc<Mutex<HashMap<u32, Vec<u8>>>> = Arc::default();
        let had: Arc<Mutex<Vec<String>>> = Arc::default();
        let slowed: Arc<Mutex<std::collections::HashSet<String>>> = Arc::default();
        let log = had.clone();
        tokio::spawn(async move {
            while let Ok((mut conn, _)) = listener.accept().await {
                let (objects, parts, log, slowed) =
                    (objects.clone(), parts.clone(), log.clone(), slowed.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = vec![0u8; 1 << 16];
                    let end = loop {
                        let n = match conn.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => n,
                        };
                        buf.extend(chunk.iter().take(n));
                        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                    };
                    let head =
                        String::from_utf8_lossy(buf.get(..end).unwrap_or_default()).to_string();
                    let header = |name: &str| {
                        head.lines()
                            .find_map(|l| {
                                l.split_once(':')
                                    .filter(|(n, _)| n.eq_ignore_ascii_case(name))
                            })
                            .map(|(_, v)| v.trim().to_owned())
                    };
                    let length: usize = header("content-length")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    while buf.len() < end + length {
                        match conn.read(&mut chunk).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => buf.extend(chunk.iter().take(n)),
                        }
                    }
                    let body = buf.get(end..).unwrap_or_default().to_vec();
                    let mut first = head.lines().next().unwrap_or_default().split(' ');
                    let (method, target) = (
                        first.next().unwrap_or_default(),
                        first.next().unwrap_or_default(),
                    );
                    let (path, query) = target.split_once('?').unwrap_or((target, ""));
                    if let Ok(mut log) = log.lock() {
                        log.push(format!("{method} {target}"));
                    }
                    let signed = header("authorization")
                        .is_some_and(|a| a.starts_with("AWS4-HMAC-SHA256 Credential=AKID/"));
                    let slow = path.contains("slow")
                        && slowed
                            .lock()
                            .is_ok_and(|mut s| s.insert(format!("{method} {target}")));
                    let (status, etag, answer): (&str, Option<String>, Vec<u8>) = if !signed {
                        (
                            "403 Forbidden",
                            None,
                            b"<Error><Code>AccessDenied</Code></Error>".to_vec(),
                        )
                    } else if slow {
                        (
                            "503 Slow Down",
                            None,
                            b"<Error><Code>SlowDown</Code></Error>".to_vec(),
                        )
                    } else {
                        let Ok(mut objects) = objects.lock() else {
                            return;
                        };
                        let Ok(mut parts) = parts.lock() else { return };
                        match (method, query) {
                            ("PUT", "") => {
                                objects.insert(path.to_owned(), body);
                                ("200 OK", Some("\"whole\"".into()), Vec::new())
                            }
                            ("POST", "uploads") => ("200 OK", None, b"<InitiateMultipartUploadResult><UploadId>u1</UploadId></InitiateMultipartUploadResult>".to_vec()),
                            ("PUT", q) if q.starts_with("partNumber=") => {
                                let n: u32 = q.trim_start_matches("partNumber=").split('&').next().and_then(|n| n.parse().ok()).unwrap_or(0);
                                parts.insert(n, body);
                                ("200 OK", Some(format!("\"p{n}\"")), Vec::new())
                            }
                            ("POST", "uploadId=u1") if path.contains("drop") => return,
                            ("POST", "uploadId=u1") => {
                                let mut whole = Vec::new();
                                for n in 1..=parts.len() as u32 {
                                    whole.extend(parts.remove(&n).unwrap_or_default());
                                }
                                objects.insert(path.to_owned(), whole);
                                ("200 OK", None, b"<CompleteMultipartUploadResult><Location>x</Location></CompleteMultipartUploadResult>".to_vec())
                            }
                            ("GET", "") => match objects.get(path) {
                                Some(o) => ("200 OK", None, o.clone()),
                                None => ("404 Not Found", None, b"<Error><Code>NoSuchKey</Code></Error>".to_vec()),
                            },
                            ("DELETE", "") => {
                                objects.remove(path);
                                ("204 No Content", None, Vec::new())
                            }
                            _ => ("400 Bad Request", None, Vec::new()),
                        }
                    };
                    let mut reply = format!(
                        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n",
                        answer.len()
                    );
                    if let Some(e) = etag {
                        reply.push_str(&format!("etag: {e}\r\n"));
                    }
                    reply.push_str("\r\n");
                    let _ = conn.write_all(reply.as_bytes()).await;
                    let _ = conn.write_all(&answer).await;
                    let _ = conn.shutdown().await;
                });
            }
        });
        Ok((format!("http://{addr}"), had))
    }

    /// A snapshot goes into a bucket of the host's own, in one PUT or in
    /// parts, comes back whole, and goes; a key that isn't right is
    /// refused, and the host says why until it works again.
    #[tokio::test]
    async fn snapshots_go_to_the_bucket_and_come_back() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let secret = dir.path().join("secret");
        std::fs::write(&secret, "s3cr3t\n")?;
        let cfg = |id: &str| SnapshotsConfig {
            url: Some("s3://acme-fctl/sessions".into()),
            endpoint: Some(String::new()),
            region: Some("us-east-1".into()),
            access_key_id: Some(id.into()),
            secret_access_key_file: Some(secret.clone()),
            max_mb: None,
        };
        let (endpoint, had) = stand_in().await?;
        let open = |id: &str| {
            let mut c = cfg(id);
            c.endpoint = Some(endpoint.clone());
            c
        };
        let mut store = Store::open(&open("AKID"))
            .await?
            .ok_or_else(|| anyhow::anyhow!("no store"))?;
        assert_eq!(store.max_bytes, 2048 * 1024 * 1024);
        store.first_wait = Duration::from_millis(5);
        let tries = |request: &str| {
            had.lock()
                .map(|h| h.iter().filter(|r| r.as_str() == request).count())
                .unwrap_or(0)
        };
        for (name, size) in [
            ("small", 1000usize),
            ("large", (SINGLE_PUT_MAX as usize) + PART_BYTES / 2),
        ] {
            let path = dir.path().join(name);
            let bytes: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            std::fs::write(&path, &bytes)?;
            let key = store.bucket.key("ses_1", name);
            store
                .put(&key, &path, &hex::encode(Sha256::digest(&bytes)))
                .await
                .map_err(anyhow::Error::msg)?;
            let back = dir.path().join(format!("{name}.back"));
            let got = store
                .get(&key, &back, size as u64)
                .await
                .map_err(anyhow::Error::msg)?;
            assert_eq!(got, size as u64);
            assert_eq!(std::fs::read(&back)?, bytes, "{name}");
            store.delete(&key).await.map_err(anyhow::Error::msg)?;
            assert!(store.get(&key, &back, size as u64).await.is_err());
        }
        assert_eq!(store.report().error, None);
        // A bucket that asks it to slow down gets each request again, the
        // body with it, and nothing fails.
        for (name, size) in [
            ("slow-small", 1000usize),
            ("slow-large", (SINGLE_PUT_MAX as usize) + PART_BYTES / 2),
        ] {
            let path = dir.path().join(name);
            let bytes: Vec<u8> = (0..size).map(|i| (i % 241) as u8).collect();
            std::fs::write(&path, &bytes)?;
            let key = store.bucket.key("ses_1", name);
            store
                .put(&key, &path, &hex::encode(Sha256::digest(&bytes)))
                .await
                .map_err(anyhow::Error::msg)?;
            let back = dir.path().join(format!("{name}.back"));
            let got = store
                .get(&key, &back, size as u64)
                .await
                .map_err(anyhow::Error::msg)?;
            assert_eq!(got, size as u64);
            assert_eq!(std::fs::read(&back)?, bytes, "{name}");
            store.delete(&key).await.map_err(anyhow::Error::msg)?;
        }
        assert_eq!(store.report().error, None);
        for request in [
            "PUT /acme-fctl/sessions/ses_1/slow-small",
            "GET /acme-fctl/sessions/ses_1/slow-small",
            "DELETE /acme-fctl/sessions/ses_1/slow-small",
            "POST /acme-fctl/sessions/ses_1/slow-large?uploads",
            "PUT /acme-fctl/sessions/ses_1/slow-large?partNumber=2&uploadId=u1",
            "POST /acme-fctl/sessions/ses_1/slow-large?uploadId=u1",
            "GET /acme-fctl/sessions/ses_1/slow-large",
        ] {
            assert_eq!(tries(request), 2, "{request}");
        }
        // An upload in parts whose completion gets no answer, however many
        // times it asks, is let go of, so that its parts don't stay in the
        // bucket.
        let large = dir.path().join("large");
        let dropped = store
            .put("sessions/ses_1/drop", &large, sigv4::EMPTY_SHA256)
            .await;
        assert!(dropped.is_err_and(|e| e.contains("didn't answer")));
        assert_eq!(
            tries("POST /acme-fctl/sessions/ses_1/drop?uploadId=u1"),
            TRIES as usize
        );
        assert_eq!(
            tries("DELETE /acme-fctl/sessions/ses_1/drop?uploadId=u1"),
            1
        );
        // Another key: refused, and said.
        let wrong = Store::open(&open("OTHER"))
            .await?
            .ok_or_else(|| anyhow::anyhow!("no store"))?;
        let path = dir.path().join("small");
        let refused = wrong
            .put("sessions/ses_1/x", &path, sigv4::EMPTY_SHA256)
            .await;
        assert!(refused.is_err_and(|e| e.contains("AccessDenied")));
        // A refusal isn't asked again.
        assert_eq!(tries("PUT /acme-fctl/sessions/ses_1/x"), 1);
        assert!(
            wrong
                .report()
                .error
                .is_some_and(|e| e.contains("AccessDenied"))
        );
        // A URL it can't read stops it, rather than send snapshots elsewhere.
        let mut bad = cfg("AKID");
        bad.url = Some("s3://Not A Bucket".into());
        assert!(Store::open(&bad).await.is_err());
        Ok(())
    }

    #[test]
    fn tries_again_what_the_bucket_asks_it_to() {
        use http::StatusCode;
        for (status, answer) in [
            (
                StatusCode::SERVICE_UNAVAILABLE,
                &b"<Error><Code>SlowDown</Code></Error>"[..],
            ),
            (StatusCode::INTERNAL_SERVER_ERROR, b""),
            (StatusCode::TOO_MANY_REQUESTS, b""),
            (
                StatusCode::BAD_REQUEST,
                b"<Error><Code>RequestTimeout</Code></Error>",
            ),
            (StatusCode::OK, b"<Error><Code>InternalError</Code></Error>"),
        ] {
            assert!(try_again(status, answer), "{status}");
        }
        for (status, answer) in [
            (
                StatusCode::FORBIDDEN,
                &b"<Error><Code>AccessDenied</Code></Error>"[..],
            ),
            (
                StatusCode::NOT_FOUND,
                b"<Error><Code>NoSuchKey</Code></Error>",
            ),
            (
                StatusCode::BAD_REQUEST,
                b"<Error><Code>InvalidDigest</Code></Error>",
            ),
            (StatusCode::OK, b"<CompleteMultipartUploadResult/>"),
        ] {
            assert!(!try_again(status, answer), "{status}");
        }
    }

    #[test]
    fn requests_go_where_the_bucket_is() -> anyhow::Result<()> {
        assert_eq!(
            endpoint(None, "acme-fctl", "eu-north-1")?,
            Endpoint {
                scheme: "https".into(),
                host: "acme-fctl.s3.eu-north-1.amazonaws.com".into(),
                path_style: false,
            }
        );
        assert_eq!(
            endpoint(None, "acme.fctl", "eu-north-1")?.host,
            "s3.eu-north-1.amazonaws.com"
        );
        let r2 = endpoint(
            Some("https://abc.r2.cloudflarestorage.com"),
            "acme-fctl",
            "auto",
        )?;
        assert_eq!(
            (r2.host.as_str(), r2.path_style),
            ("abc.r2.cloudflarestorage.com", true)
        );
        let minio = endpoint(Some("http://127.0.0.1:9000"), "acme-fctl", "us-east-1")?;
        assert_eq!(
            (minio.scheme.as_str(), minio.host.as_str()),
            ("http", "127.0.0.1:9000")
        );
        Ok(())
    }

    #[test]
    fn reads_keys_and_answers() {
        let json = br#"{"Code":"Success","AccessKeyId":"AKIA","SecretAccessKey":"s3cr3t","Token":"t","Expiration":"2026-11-23T12:00:00Z"}"#;
        let keys = keys_from_json(json);
        assert_eq!(
            keys.as_ref().map(|k| k.access_key_id.as_str()),
            Some("AKIA")
        );
        assert_eq!(keys.and_then(|k| k.session_token).as_deref(), Some("t"));
        let xml =
            b"<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult><Credentials>\
            <SessionToken>st</SessionToken><SecretAccessKey>sk</SecretAccessKey>\
            <Expiration>2026-11-23T12:00:00Z</Expiration><AccessKeyId>ASIA</AccessKeyId>\
            </Credentials></AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>";
        let keys = keys_from_xml(xml);
        assert_eq!(
            keys.as_ref().map(|k| k.access_key_id.as_str()),
            Some("ASIA")
        );
        assert_eq!(keys.and_then(|k| k.session_token).as_deref(), Some("st"));
        assert_eq!(
            complete_body(&["\"a\"".into(), "\"b\"".into()]),
            "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>\"a\"</ETag></Part>\
             <Part><PartNumber>2</PartNumber><ETag>\"b\"</ETag></Part></CompleteMultipartUpload>"
        );
        assert_eq!(
            expect_ok(
                http::StatusCode::FORBIDDEN,
                b"<Error><Code>AccessDenied</Code></Error>",
                "putting it"
            ),
            Err("putting it failed: AccessDenied".into())
        );
    }
}
