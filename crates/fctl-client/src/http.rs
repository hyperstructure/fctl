//! A small HTTPS client: JSON requests with timeouts, and downloads.

use bytes::Bytes;
use fctl_proto::ApiError;
use http::{HeaderValue, Method, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;

/// Whether `link` is on the site of `server`: the same origin, or both on
/// this machine (a development control plane and its web app run on
/// different ports). A server that sends people elsewhere to approve a
/// sign-in may be relaying another server's code, to get a token there.
pub fn same_site(server: &str, link: &str) -> bool {
    let (Ok(server), Ok(link)) = (url::Url::parse(server), url::Url::parse(link)) else {
        return false;
    };
    let local = |u: &url::Url| match u.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    server.origin() == link.origin() || (local(&server) && local(&link))
}

/// The origin of a URL, as it can be shown: `https://app.fctl.sh`.
pub fn origin(link: &str) -> Option<String> {
    let origin = url::Url::parse(link).ok()?.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The server answered with an fctl error body.
    #[error("{0}")]
    Api(ApiError),
    #[error("HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("could not reach {url}: {message}")]
    Network { url: String, message: String },
    #[error("request timed out after {0:?}")]
    Timeout(Duration),
    #[error("invalid response: {0}")]
    Decode(String),
    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn api(&self) -> Option<&ApiError> {
        match self {
            Self::Api(e) => Some(e),
            _ => None,
        }
    }

    /// Whether retrying later may succeed.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Network { .. } | Self::Timeout(_) => true,
            Self::Status { status, .. } => *status >= 500 || *status == 429,
            Self::Api(e) => matches!(
                e.code,
                fctl_proto::ErrorCode::Unavailable
                    | fctl_proto::ErrorCode::RateLimited
                    | fctl_proto::ErrorCode::Internal
            ),
            _ => false,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// How many times [`retrying`] tries in all.
pub const TRIES: u32 = 4;

/// How long [`retrying`] waits before its second try; three times as long
/// before each one after it (1, 3 and 9 seconds).
pub const FIRST_WAIT: Duration = Duration::from_secs(1);

/// How long after its first try [`retrying`] may start another, with the
/// wait before it: it bounds when a try starts, not when the last one ends.
/// A control plane that doesn't answer at all gets one more try over HTTP,
/// whose requests time out after 30 seconds (the second ends near 61
/// seconds), and none through an autopilot's relay, whose calls fctl-agent
/// gives up on after two minutes, so that a caller with a time limit of its
/// own (an agent's tool call) hears of it.
pub const TRY_WITHIN: Duration = Duration::from_secs(60);

/// Runs `attempt` until it succeeds, fails with an error that trying again
/// wouldn't change, or has failed [`TRIES`] times, or too late to start
/// another try ([`TRY_WITHIN`]), with errors that may pass ([`Error::is_transient`]): a hiccup of the
/// control plane's database, a lost connection. Only for requests that
/// change nothing when they come twice, such as a message with a
/// `client_id`, which a request whose answer was lost may have delivered.
/// `retried` hears of each error tried again, with how long it waits first.
pub async fn retrying<T, F, Fut>(
    mut attempt: F,
    mut retried: impl FnMut(&Error, Duration),
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let started = tokio::time::Instant::now();
    let mut wait = FIRST_WAIT;
    let mut tries = 1;
    loop {
        match attempt().await {
            Err(e)
                if e.is_transient() && tries < TRIES && started.elapsed() + wait <= TRY_WITHIN =>
            {
                retried(&e, wait);
                tokio::time::sleep(wait).await;
                wait *= 3;
                tries += 1;
            }
            done => return done,
        }
    }
}

type Connector = hyper_rustls::HttpsConnector<HttpConnector>;

#[derive(Clone)]
pub struct HttpClient {
    client: Client<Connector, Full<Bytes>>,
    timeout: Duration,
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}

impl HttpClient {
    pub fn new(timeout: Duration) -> Self {
        let mut http = HttpConnector::new();
        http.enforce_http(false);
        http.set_connect_timeout(Some(Duration::from_secs(10)));
        http.set_keepalive(Some(Duration::from_secs(60)));
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config((*crate::tls_config()).clone())
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .wrap_connector(http);
        let client = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(60))
            .build(https);
        Self { client, timeout }
    }

    /// Sends a request; returns status, headers and body.
    pub async fn send(
        &self,
        method: Method,
        url: &str,
        token: Option<&str>,
        body: Option<Vec<u8>>,
        content_type: Option<&str>,
    ) -> Result<(StatusCode, http::HeaderMap, Bytes)> {
        let req = request(method, url, token, body, content_type, &[])?;
        self.finish(url, req).await
    }

    /// A GET with `headers` besides the usual ones, which they replace (an
    /// `Accept` of their own, say).
    pub async fn get_with(
        &self,
        url: &str,
        token: Option<&str>,
        headers: &[(&str, &str)],
    ) -> Result<(StatusCode, http::HeaderMap, Bytes)> {
        let req = request(Method::GET, url, token, None, None, headers)?;
        self.finish(url, req).await
    }

    /// Downloads `url` into the file at `path`, following up to 5
    /// redirects, within `limit`. The token and `headers` go to `url`'s
    /// origin only. Fails once more than `max_bytes` came. Returns the size.
    pub async fn download_to(
        &self,
        url: &str,
        token: Option<&str>,
        headers: &[(&str, &str)],
        path: &std::path::Path,
        max_bytes: u64,
        limit: Duration,
    ) -> Result<u64> {
        tokio::time::timeout(
            limit,
            self.download_to_inner(url, token, headers, path, max_bytes),
        )
        .await
        .map_err(|_| Error::Timeout(limit))?
    }

    async fn download_to_inner(
        &self,
        url: &str,
        token: Option<&str>,
        headers: &[(&str, &str)],
        path: &std::path::Path,
        max_bytes: u64,
    ) -> Result<u64> {
        use tokio::io::AsyncWriteExt;
        let mut body = self.open(url, token, headers).await?;
        let io = |e: std::io::Error| Error::Other(format!("writing {}: {e}", path.display()));
        let mut file = tokio::fs::File::create(path).await.map_err(io)?;
        let mut size: u64 = 0;
        while let Some(data) = body.next().await? {
            size += data.len() as u64;
            if size > max_bytes {
                return Err(Error::Other(format!(
                    "{} is larger than {} MiB",
                    body.url,
                    max_bytes / 1024 / 1024
                )));
            }
            file.write_all(&data).await.map_err(io)?;
        }
        file.flush().await.map_err(io)?;
        Ok(size)
    }

    /// GETs `url`, following up to 5 redirects, for its body to be read as
    /// it comes ([`Body::next`]), with no time limit: the caller sets one.
    /// The token and `headers` go to `url`'s origin only.
    pub async fn open(
        &self,
        url: &str,
        token: Option<&str>,
        headers: &[(&str, &str)],
    ) -> Result<Body> {
        self.open_from(url, token, headers, 0).await
    }

    /// [`open`](Self::open), for the bytes of `url` from `from` on, to go on
    /// with a download that stopped there: the `Range` that asks for them
    /// goes with every redirect (a registry's blobs are on a CDN). A server
    /// may send all of it anyway: [`Body::start`] says where the body
    /// starts.
    pub async fn open_from(
        &self,
        url: &str,
        token: Option<&str>,
        headers: &[(&str, &str)],
        from: u64,
    ) -> Result<Body> {
        let origin = |u: &str| url::Url::parse(u).ok().map(|u| u.origin());
        let first = origin(url);
        let range = format!("bytes={from}-");
        let mut current = url.to_owned();
        for _ in 0..6 {
            let same_origin = first.is_some() && origin(&current) == first;
            let mut sent: Vec<(&str, &str)> = if same_origin {
                headers.to_vec()
            } else {
                Vec::new()
            };
            let token = token.filter(|_| same_origin);
            if from > 0 {
                sent.push(("range", &range));
            }
            let req = request(Method::GET, &current, token, None, None, &sent)?;
            // Signed download links carry a token in their query.
            let shown = without_query(&current);
            let res = self.client.request(req).await.map_err(|e| Error::Network {
                url: shown.clone(),
                message: error_chain(&e),
            })?;
            let status = res.status();
            if status.is_redirection() {
                let Some(loc) = res
                    .headers()
                    .get(http::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                else {
                    return Err(Error::Decode("redirect without a location".into()));
                };
                current = url::Url::parse(&current)
                    .and_then(|base| base.join(loc))
                    .map(|u| u.to_string())
                    .map_err(|e| Error::Decode(e.to_string()))?;
                continue;
            }
            let start = if status == StatusCode::PARTIAL_CONTENT {
                res.headers()
                    .get(http::header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(range_start)
                    .ok_or_else(|| Error::Decode("a partial answer without its range".into()))?
            } else {
                0
            };
            let body = res.into_body();
            if !status.is_success() {
                let bytes = body
                    .collect()
                    .await
                    .map(|b| b.to_bytes())
                    .unwrap_or_default();
                return Err(Error::Status {
                    status: status.as_u16(),
                    body: String::from_utf8_lossy(&bytes).chars().take(500).collect(),
                });
            }
            return Ok(Body {
                body,
                url: shown,
                start,
            });
        }
        Err(Error::Other("too many redirects".into()))
    }

    async fn finish(
        &self,
        url: &str,
        req: Request<Full<Bytes>>,
    ) -> Result<(StatusCode, http::HeaderMap, Bytes)> {
        let fut = async {
            let res = self.client.request(req).await.map_err(|e| Error::Network {
                url: url.to_owned(),
                message: error_chain(&e),
            })?;
            let (parts, body) = res.into_parts();
            let bytes = body
                .collect()
                .await
                .map_err(|e| Error::Network {
                    url: url.to_owned(),
                    message: e.to_string(),
                })?
                .to_bytes();
            Ok((parts.status, parts.headers, bytes))
        };
        tokio::time::timeout(self.timeout, fut)
            .await
            .map_err(|_| Error::Timeout(self.timeout))?
    }

    /// A JSON request that decodes a JSON answer (or an fctl error).
    pub async fn json<B: Serialize, T: DeserializeOwned>(
        &self,
        method: Method,
        url: &str,
        token: Option<&str>,
        body: Option<&B>,
    ) -> Result<T> {
        let payload = match body {
            Some(b) => Some(serde_json::to_vec(b).map_err(|e| Error::Other(e.to_string()))?),
            None => None,
        };
        let ct = payload.as_ref().map(|_| "application/json");
        let (status, _, bytes) = self.send(method, url, token, payload, ct).await?;
        if !status.is_success() {
            if let Ok(e) = serde_json::from_slice::<ApiError>(&bytes) {
                return Err(Error::Api(e));
            }
            return Err(Error::Status {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).chars().take(500).collect(),
            });
        }
        serde_json::from_slice(&bytes).map_err(|e| Error::Decode(e.to_string()))
    }

    /// Downloads `url` into memory (follows up to 5 redirects).
    pub async fn download(&self, url: &str) -> Result<Vec<u8>> {
        let mut url = url.to_owned();
        for _ in 0..5 {
            let (status, headers, bytes) = self.send(Method::GET, &url, None, None, None).await?;
            if status.is_redirection() {
                let Some(loc) = headers
                    .get(http::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                else {
                    return Err(Error::Decode("redirect without a location".into()));
                };
                url = url::Url::parse(&url)
                    .and_then(|base| base.join(loc))
                    .map(|u| u.to_string())
                    .map_err(|e| Error::Decode(e.to_string()))?;
                continue;
            }
            if !status.is_success() {
                return Err(Error::Status {
                    status: status.as_u16(),
                    body: String::new(),
                });
            }
            return Ok(bytes.to_vec());
        }
        Err(Error::Other("too many redirects".into()))
    }
}

/// The body of a response to [`HttpClient::open`], read as it comes.
pub struct Body {
    body: hyper::body::Incoming,
    /// Where it comes from, without the query, which may hold a token.
    pub url: String,
    /// Where in what `url` has the body starts: 0, unless the server sent
    /// only a part ([`HttpClient::open_from`]).
    pub start: u64,
}

impl Body {
    /// What came next; `None` once all of it came.
    pub async fn next(&mut self) -> Result<Option<Bytes>> {
        while let Some(frame) = self.body.frame().await {
            let frame = frame.map_err(|e| Error::Network {
                url: self.url.clone(),
                message: error_chain(&e),
            })?;
            if let Ok(data) = frame.into_data()
                && !data.is_empty()
            {
                return Ok(Some(data));
            }
        }
        Ok(None)
    }
}

/// Where the part in a `Content-Range` (`bytes 100-199/200`) starts.
fn range_start(header: &str) -> Option<u64> {
    let (first, _) = header.trim().strip_prefix("bytes ")?.split_once('-')?;
    first.trim().parse().ok()
}

/// `url` without its query or fragment, for messages.
fn without_query(url: &str) -> String {
    url.split(['?', '#']).next().unwrap_or_default().to_owned()
}

/// A request with fctl's user agent, JSON accepted, the token, and
/// `headers`, which replace those.
fn request(
    method: Method,
    url: &str,
    token: Option<&str>,
    body: Option<Vec<u8>>,
    content_type: Option<&str>,
    headers: &[(&str, &str)],
) -> Result<Request<Full<Bytes>>> {
    let mut req = Request::builder().method(method).uri(url);
    if let Some(h) = req.headers_mut() {
        if let Ok(v) = HeaderValue::from_str(&crate::user_agent()) {
            h.insert(http::header::USER_AGENT, v);
        }
        h.insert(
            http::header::ACCEPT,
            HeaderValue::from_static("application/json"),
        );
        if let Some(t) = token
            && let Ok(v) = HeaderValue::from_str(&format!("Bearer {t}"))
        {
            h.insert(http::header::AUTHORIZATION, v);
        }
        if let Some(ct) = content_type
            && let Ok(v) = HeaderValue::from_str(ct)
        {
            h.insert(http::header::CONTENT_TYPE, v);
        }
        for (name, value) in headers {
            if let (Ok(name), Ok(value)) = (
                http::header::HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                h.insert(name, value);
            }
        }
    }
    req.body(Full::new(Bytes::from(body.unwrap_or_default())))
        .map_err(|e| Error::Other(e.to_string()))
}

fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        s.push_str(": ");
        s.push_str(&c.to_string());
        cur = c.source();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_links_stay_on_the_server() {
        assert!(same_site(
            "https://app.fctl.sh",
            "https://app.fctl.sh/device?code=AB"
        ));
        assert!(same_site(
            "http://127.0.0.1:8787",
            "http://localhost:5173/device?code=AB"
        ));
        assert!(!same_site(
            "https://evil.example",
            "https://app.fctl.sh/device"
        ));
        assert!(!same_site(
            "https://app.fctl.sh",
            "http://app.fctl.sh/device"
        ));
        assert!(!same_site(
            "https://app.fctl.sh",
            "https://app.fctl.sh.evil.example/device"
        ));
        assert!(!same_site(
            "http://127.0.0.1:8787",
            "https://app.fctl.sh/device"
        ));
        assert!(!same_site("https://app.fctl.sh", "javascript:alert(1)"));
        assert_eq!(
            origin("https://app.fctl.sh/device?code=AB").as_deref(),
            Some("https://app.fctl.sh")
        );
        assert_eq!(origin("javascript:alert(1)"), None);
    }

    #[test]
    fn reads_where_a_part_starts() {
        assert_eq!(range_start("bytes 100-199/200"), Some(100));
        assert_eq!(range_start("bytes 0-9/*"), Some(0));
        assert_eq!(range_start("bytes */200"), None);
        assert_eq!(range_start("items 1-2/3"), None);
    }

    fn internal() -> Error {
        Error::Api(ApiError::internal(
            "Something went wrong on our side. Try again.",
        ))
    }

    /// Runs [`retrying`] on `answers`, one per try, on a paused clock:
    /// what it returned, how many tries it made and the waits it took.
    async fn retry(answers: Vec<Result<u32>>) -> (Result<u32>, usize, Vec<Duration>) {
        let answers = std::sync::Mutex::new(answers.into_iter());
        let tries = std::sync::atomic::AtomicUsize::new(0);
        let mut waits = Vec::new();
        let out = retrying(
            || async {
                tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let next = answers.lock().ok().and_then(|mut a| a.next());
                next.unwrap_or_else(|| Err(Error::Other("no answer left".into())))
            },
            |_, wait| waits.push(wait),
        )
        .await;
        (out, tries.into_inner(), waits)
    }

    #[tokio::test(start_paused = true)]
    async fn tries_again_while_errors_may_pass() {
        let started = tokio::time::Instant::now();
        let (out, tries, waits) = retry(vec![
            Err(internal()),
            Err(Error::Timeout(Duration::from_secs(30))),
            Ok(7),
        ])
        .await;
        assert_eq!(out.ok(), Some(7));
        assert_eq!(tries, 3);
        assert_eq!(waits, [Duration::from_secs(1), Duration::from_secs(3)]);
        assert_eq!(started.elapsed(), Duration::from_secs(4));
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_after_its_last_try() {
        let (out, tries, waits) = retry(vec![
            Err(internal()),
            Err(internal()),
            Err(internal()),
            Err(Error::Status {
                status: 503,
                body: "unavailable".into(),
            }),
            Ok(7),
        ])
        .await;
        assert!(out.is_err_and(|e| matches!(e, Error::Status { status: 503, .. })));
        assert_eq!(tries, TRIES as usize);
        assert_eq!(
            waits,
            [1, 3, 9].map(Duration::from_secs),
            "it waits longer each time"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tries_again_within_a_minute() {
        // Each try waits 50 seconds for an answer that doesn't come.
        let tries = std::sync::atomic::AtomicUsize::new(0);
        let started = tokio::time::Instant::now();
        let out: Result<()> = retrying(
            || async {
                tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                tokio::time::sleep(Duration::from_secs(50)).await;
                Err(Error::Timeout(Duration::from_secs(50)))
            },
            |_, _| {},
        )
        .await;
        assert!(out.is_err_and(|e| matches!(e, Error::Timeout(_))));
        assert_eq!(tries.into_inner(), 2);
        assert_eq!(started.elapsed(), Duration::from_secs(101));
    }

    #[tokio::test(start_paused = true)]
    async fn answers_at_once_what_trying_again_wouldnt_change() {
        let refused = Error::Api(ApiError::new(
            fctl_proto::ErrorCode::Conflict,
            "The session failed. Resume it to continue.",
        ));
        let (out, tries, waits) = retry(vec![Err(refused), Ok(7)]).await;
        assert!(out.is_err_and(|e| e.api().is_some_and(|e| e.message.contains("failed"))));
        assert_eq!(tries, 1);
        assert!(waits.is_empty());
        let (out, tries, _) = retry(vec![Ok(7)]).await;
        assert_eq!((out.ok(), tries), (Some(7), 1));
    }
}
