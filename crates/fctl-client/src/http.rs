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
        let origin = |u: &str| url::Url::parse(u).ok().map(|u| u.origin());
        let first = origin(url);
        let mut current = url.to_owned();
        for _ in 0..6 {
            let same_origin = first.is_some() && origin(&current) == first;
            let (token, headers) = if same_origin {
                (token, headers)
            } else {
                (None, &[][..])
            };
            let req = request(Method::GET, &current, token, None, None, headers)?;
            let network = |message: String| Error::Network {
                url: current.clone(),
                message,
            };
            let res = self
                .client
                .request(req)
                .await
                .map_err(|e| network(error_chain(&e)))?;
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
            let mut body = res.into_body();
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
            let io = |e: std::io::Error| Error::Other(format!("writing {}: {e}", path.display()));
            let mut file = tokio::fs::File::create(path).await.map_err(io)?;
            let mut size: u64 = 0;
            while let Some(frame) = body.frame().await {
                let frame = frame.map_err(|e| network(e.to_string()))?;
                if let Ok(data) = frame.into_data() {
                    size += data.len() as u64;
                    if size > max_bytes {
                        return Err(Error::Other(format!(
                            "{current} is larger than {} MiB",
                            max_bytes / 1024 / 1024
                        )));
                    }
                    file.write_all(&data).await.map_err(io)?;
                }
            }
            file.flush().await.map_err(io)?;
            return Ok(size);
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
