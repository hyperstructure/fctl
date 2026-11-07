//! Keeps Claude tokens out of sandboxes, for the people who ask for it
//! ([`ClaudeCredential::stand_in`](fctl_proto::runner::ClaudeCredential::stand_in)).
//!
//! Claude in such a sandbox has a stand-in token ([`CLAUDE_STAND_IN`]),
//! which Anthropic refuses, and an HTTPS proxy, fctl-agent's, which hands
//! each of its connections to the Anthropic API to the host: the agent asks
//! for one ([`AgentToHost::ClaudeConnection`]), and the host opens a channel
//! to the sandbox's relay socket, as it attaches to fctl-agent. On that
//! channel the host answers Claude's TLS as [`CLAUDE_API_HOST`], with a
//! certificate of its own authority ([`Authority`]), which the sandbox's
//! Claude trusts and nothing else does, reads each request, puts the
//! session's token in place of the stand-in, and sends the request on to
//! the Anthropic API ([`Upstream`]), checking its certificate as it checks
//! any other. So the token goes to the Anthropic API alone, from the host,
//! and nothing in the sandbox ever has it: an agent that is talked into
//! handing it over has only the stand-in to give.
//!
//! [`AgentToHost::ClaudeConnection`]: fctl_proto::agent::AgentToHost::ClaudeConnection

use anyhow::{Context, Result};
use bytes::Bytes;
use fctl_proto::agent::CLAUDE_API_HOST;
use fctl_proto::runner::CLAUDE_STAND_IN;
use http::header::{self, HeaderMap, HeaderName, HeaderValue};
use http::{Method, Request, Response, StatusCode, Uri};
use http_body_util::{Either, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
    GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose, NameConstraints,
};
use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::convert::Infallible;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use time::OffsetDateTime;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;

/// The host's certificate authority for Claude's connections: its
/// certificate, then its key, PEM, readable only by the host's user. In the
/// directory of `host.toml`, beside the runner's token.
pub const FILE: &str = "claude-relay-ca.pem";

/// What the authority calls itself, which every certificate it signs names
/// as its issuer.
const AUTHORITY_NAME: &str = "fctl host: Claude's connections to api.anthropic.com";

/// How long the authority is good for. Sandboxes trust it for as long as
/// they run, and a host keeps it for as long as it is installed.
const AUTHORITY_DAYS: i64 = 20 * 365;

/// How long a certificate the host answers with is good for, and how long
/// before its end the host makes the next one.
const CERTIFICATE_DAYS: i64 = 90;
const RENEW_DAYS: i64 = 7;

/// How long Claude has to start TLS on a channel the host opened for it.
const HANDSHAKE: Duration = Duration::from_secs(30);

/// How long a connection may wait for its next request. Claude opens
/// another when it needs one.
const IDLE: Duration = Duration::from_secs(120);

/// How long a request waits for the session's token, which a host that
/// restarted asks the control plane for again.
const TOKEN_WAIT: Duration = Duration::from_secs(60);

/// The host's certificate authority: it signs the certificate the host
/// answers Claude's connections with, for [`CLAUDE_API_HOST`] alone (its
/// name constraints say so too, for the TLS clients that heed them), and
/// nothing else. Its key never leaves the host.
pub struct Authority {
    /// Its certificate, PEM: what sandboxes trust.
    pem: String,
    der: CertificateDer<'static>,
    key: KeyPair,
    /// The TLS configuration the host answers with, and until when its
    /// certificate is good (seconds since the epoch).
    server: Mutex<Option<(Arc<ServerConfig>, i64)>>,
}

impl Authority {
    /// The authority kept at `path`, or a new one, kept there for the next
    /// time (mode 0600). One that can't be read is replaced: the sandboxes
    /// that trusted it can no longer reach the Anthropic API, until their
    /// Claude starts again.
    pub fn load_or_make(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => match Self::parse(&text) {
                Ok(a) => return Ok(a),
                Err(e) => tracing::warn!(
                    "{} holds no certificate authority fctl can use ({e:#}); making a new one",
                    path.display()
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        }
        let key = KeyPair::generate().context("making the authority's key")?;
        let now = OffsetDateTime::now_utc();
        let mut params = authority_params();
        params.not_before = now - time::Duration::days(1);
        params.not_after = now + time::Duration::days(AUTHORITY_DAYS);
        let cert = params
            .self_signed(&key)
            .context("signing the authority's certificate")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::config::write_private(path, &format!("{}{}", cert.pem(), key.serialize_pem()))
            .with_context(|| format!("writing {}", path.display()))?;
        tracing::info!(
            "made a certificate authority for Claude's connections at {}",
            path.display()
        );
        Ok(Self {
            pem: cert.pem(),
            der: cert.der().clone(),
            key,
            server: Mutex::new(None),
        })
    }

    fn parse(text: &str) -> Result<Self> {
        let der = CertificateDer::from_pem_slice(text.as_bytes())
            .context("no certificate in it")?
            .into_owned();
        let key = pem_block(text, "PRIVATE KEY").context("no key in it")?;
        let key = KeyPair::from_pem(&key).context("its key")?;
        let pem = pem_block(text, "CERTIFICATE").context("no certificate in it")?;
        Ok(Self {
            pem,
            der,
            key,
            server: Mutex::new(None),
        })
    }

    /// Its certificate, PEM, for sandboxes to trust
    /// ([`fctl_proto::agent::ClaudeRelay::ca`]).
    pub fn pem(&self) -> &str {
        &self.pem
    }

    /// How the host answers Claude's TLS: as [`CLAUDE_API_HOST`], with a
    /// certificate this authority signed, made anew as the last one nears
    /// its end.
    pub fn server(&self) -> Result<Arc<ServerConfig>> {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let mut server = self.server.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((config, until)) = server.as_ref()
            && *until - now > RENEW_DAYS * 86_400
        {
            return Ok(config.clone());
        }
        let (config, until) = self.sign()?;
        *server = Some((config.clone(), until));
        Ok(config)
    }

    fn sign(&self) -> Result<(Arc<ServerConfig>, i64)> {
        let key = KeyPair::generate().context("making the certificate's key")?;
        let now = OffsetDateTime::now_utc();
        let until = now + time::Duration::days(CERTIFICATE_DAYS);
        let mut params = CertificateParams::new(vec![CLAUDE_API_HOST.to_owned()])?;
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, CLAUDE_API_HOST);
        params.distinguished_name = name;
        params.not_before = now - time::Duration::days(1);
        params.not_after = until;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        let authority = authority_params();
        let issuer = Issuer::from_params(&authority, &self.key);
        let cert = params
            .signed_by(&key, &issuer)
            .context("signing the certificate")?;
        let chain = vec![cert.der().clone(), self.der.clone()];
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(chain, key)?;
        // Claude's requests are HTTP/1.1, which the host reads one by one.
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok((Arc::new(config), until.unix_timestamp()))
    }
}

/// The authority's own certificate, as it makes it and as it names itself
/// in those it signs.
fn authority_params() -> CertificateParams {
    let mut params = CertificateParams::default();
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, AUTHORITY_NAME);
    params.distinguished_name = name;
    // It signs the host's certificates, and no other authority's.
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.name_constraints = Some(NameConstraints {
        permitted_subtrees: vec![GeneralSubtree::DnsName(CLAUDE_API_HOST.to_owned())],
        excluded_subtrees: Vec::new(),
    });
    params
}

/// The first PEM block of `kind` in `text`, with its markers.
fn pem_block(text: &str, kind: &str) -> Option<String> {
    let begin = format!("-----BEGIN {kind}-----");
    let end = format!("-----END {kind}-----");
    let start = text.find(&begin)?;
    let stop = text[start..].find(&end)? + start + end.len();
    Some(format!("{}\n", &text[start..stop]))
}

type Connector = HttpsConnector<HttpConnector>;

/// Where the host sends Claude's requests: the Anthropic API, over TLS that
/// checks its certificate against the roots the host trusts, as anything
/// else it reaches.
#[derive(Clone)]
pub struct Upstream {
    /// `https://api.anthropic.com`
    origin: String,
    client: Client<Connector, Incoming>,
}

impl Upstream {
    /// The Anthropic API. A debug build (`cargo xtask dev`'s) reaches a
    /// stand-in instead where `FCTL_DEV_CLAUDE_API` names one
    /// (`https://localhost:PORT`, its certificate in `SSL_CERT_FILE`);
    /// release builds always reach the real one.
    pub fn anthropic() -> Self {
        let tls = (*fctl_client::tls_config()).clone();
        #[cfg(debug_assertions)]
        if let Ok(origin) = std::env::var("FCTL_DEV_CLAUDE_API") {
            tracing::warn!(
                "Claude's requests go to {origin}, not the Anthropic API (FCTL_DEV_CLAUDE_API)"
            );
            return Self::new(origin, tls);
        }
        Self::new(format!("https://{CLAUDE_API_HOST}"), tls)
    }

    /// `origin` (`https://host:port`), trusting what `tls` trusts: tests
    /// stand in for the Anthropic API with it.
    pub fn new(origin: String, tls: rustls::ClientConfig) -> Self {
        let mut http = HttpConnector::new();
        http.enforce_http(false);
        http.set_connect_timeout(Some(Duration::from_secs(10)));
        http.set_keepalive(Some(Duration::from_secs(60)));
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(tls)
            .https_only()
            .enable_http1()
            .enable_http2()
            .wrap_connector(http);
        let client = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(90))
            .build(https);
        Self { origin, client }
    }
}

/// What a host needs to relay Claude's connections: its authority and the
/// way to the Anthropic API. Made the first time a session needs it.
pub struct Relay {
    pub authority: Authority,
    pub upstream: Upstream,
}

/// The token of one session, as the host knows it: `None` until it does
/// (a host that restarted asks the control plane for it again). Its
/// channels wait for it.
#[derive(Clone)]
pub struct Token(Arc<watch::Sender<Option<String>>>);

impl Default for Token {
    fn default() -> Self {
        Self(Arc::new(watch::channel(None).0))
    }
}

impl Token {
    /// The session's token is `token` from now on.
    pub fn set(&self, token: &str) {
        self.0.send_replace(Some(token.to_owned()));
    }

    /// Whether the host knows it.
    pub fn known(&self) -> bool {
        self.0.borrow().is_some()
    }

    fn watch(&self) -> watch::Receiver<Option<String>> {
        self.0.subscribe()
    }
}

/// Answers one of Claude's connections on `io`, the channel to its sandbox's
/// relay socket, until either side closes it: TLS as [`CLAUDE_API_HOST`],
/// then each request, sent on to `upstream` with `token` in place of the
/// stand-in.
pub async fn serve<IO>(
    io: IO,
    tls: Arc<ServerConfig>,
    token: Token,
    upstream: Upstream,
) -> Result<()>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let tls = tokio::time::timeout(HANDSHAKE, tokio_rustls::TlsAcceptor::from(tls).accept(io))
        .await
        .context("Claude didn't start TLS in time")?
        .context("TLS with Claude")?;
    let relay = Arc::new(Forward {
        token: token.watch(),
        upstream,
    });
    hyper::server::conn::http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(IDLE)
        .serve_connection(
            TokioIo::new(tls),
            service_fn(move |req| {
                let relay = relay.clone();
                async move { Ok::<_, Infallible>(relay.forward(req).await) }
            }),
        )
        .await
        .context("Claude's requests")
}

type Body = Either<Incoming, Full<Bytes>>;

struct Forward {
    token: watch::Receiver<Option<String>>,
    upstream: Upstream,
}

impl Forward {
    async fn forward(&self, req: Request<Incoming>) -> Response<Body> {
        if req.method() == Method::CONNECT || req.headers().contains_key(header::UPGRADE) {
            return refusal(
                StatusCode::NOT_IMPLEMENTED,
                "fctl's host passes requests on to the Anthropic API one by one, and no upgrades",
            );
        }
        let (mut parts, body) = req.into_parts();
        let path = parts.uri.path_and_query().map_or("/", |p| p.as_str());
        let Ok(uri) = format!("{}{path}", self.upstream.origin).parse::<Uri>() else {
            return refusal(StatusCode::BAD_REQUEST, "That isn't a path");
        };
        drop_hop_by_hop(&mut parts.headers);
        // The client names the host it connects to.
        parts.headers.remove(header::HOST);
        if stands_in(&parts.headers) {
            match self.token().await {
                Some(value) => {
                    parts.headers.insert(header::AUTHORIZATION, value);
                }
                None => {
                    return refusal(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "fctl's host has no Claude token for this session: it asked the control plane for one, which didn't answer in time",
                    );
                }
            }
        }
        parts.uri = uri;
        parts.version = http::Version::HTTP_11;
        match self
            .upstream
            .client
            .request(Request::from_parts(parts, body))
            .await
        {
            Ok(res) => {
                let (mut parts, body) = res.into_parts();
                drop_hop_by_hop(&mut parts.headers);
                Response::from_parts(parts, Either::Left(body))
            }
            Err(e) => {
                let why = causes(&e);
                tracing::warn!("passing Claude's request on to the Anthropic API: {why}");
                refusal(
                    StatusCode::BAD_GATEWAY,
                    &format!("fctl's host couldn't reach the Anthropic API: {why}"),
                )
            }
        }
    }

    /// `Bearer <the session's token>`, once the host knows it.
    async fn token(&self) -> Option<HeaderValue> {
        let mut rx = self.token.clone();
        let token = tokio::time::timeout(TOKEN_WAIT, rx.wait_for(Option::is_some))
            .await
            .ok()?
            .ok()?
            .clone()?;
        let mut value = HeaderValue::from_str(&format!("Bearer {token}")).ok()?;
        value.set_sensitive(true);
        Some(value)
    }
}

/// An error with what caused it, as far down as it goes: "client error
/// (Connect): invalid peer certificate: …".
fn causes(e: &(dyn std::error::Error + 'static)) -> String {
    let mut text = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        let said = s.to_string();
        if !text.contains(&said) {
            text.push_str(": ");
            text.push_str(&said);
        }
        source = s.source();
    }
    text
}

/// Whether the request authorizes itself with the stand-in, which the host
/// replaces. Requests without it, or with any other credential, go on as
/// they are.
fn stands_in(headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| t.trim() == CLAUDE_STAND_IN)
}

/// Removes the headers that concern one connection, not the request: those
/// `Connection` names, and the usual ones.
fn drop_hop_by_hop(headers: &mut HeaderMap) {
    let named: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect();
    for name in named {
        headers.remove(name);
    }
    for name in [
        header::CONNECTION,
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
        header::TE,
        header::TRAILER,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
    ] {
        headers.remove(name);
    }
    headers.remove("keep-alive");
    headers.remove("proxy-connection");
}

/// An answer of the host's own, as the Anthropic API words its errors, so
/// that Claude says why.
fn refusal(status: StatusCode, message: &str) -> Response<Body> {
    let body = serde_json::json!({
        "type": "error",
        "error": { "type": "api_error", "message": message },
    });
    let mut res = Response::new(Either::Right(Full::new(Bytes::from(body.to_string()))));
    *res.status_mut() = status;
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn the_authority_is_kept_and_signs_for_the_anthropic_api_alone() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join(FILE);
        let made = Authority::load_or_make(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path)?.permissions().mode() & 0o777,
                0o600
            );
        }
        // The next start finds the same one.
        let kept = Authority::load_or_make(&path)?;
        assert_eq!(kept.pem(), made.pem());
        assert!(kept.pem().starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(!kept.pem().contains("PRIVATE KEY"));
        // One that can't be read is replaced.
        std::fs::write(&path, "nonsense")?;
        let replaced = Authority::load_or_make(&path)?;
        assert_ne!(replaced.pem(), made.pem());
        assert_eq!(Authority::load_or_make(&path)?.pem(), replaced.pem());
        Ok(())
    }

    #[test]
    fn only_the_stand_in_is_replaced() {
        let with = |value: &str| {
            let mut h = HeaderMap::new();
            h.insert(
                header::AUTHORIZATION,
                HeaderValue::from_str(value).unwrap_or_else(|_| HeaderValue::from_static("x")),
            );
            h
        };
        assert!(stands_in(&with(&format!("Bearer {CLAUDE_STAND_IN}"))));
        assert!(!stands_in(&with("Bearer sk-ant-oat01-somebody-elses")));
        assert!(!stands_in(&with(CLAUDE_STAND_IN)));
        assert!(!stands_in(&HeaderMap::new()));
    }

    #[test]
    fn headers_of_the_connection_stay_behind() {
        let mut h = HeaderMap::new();
        h.insert(
            header::CONNECTION,
            HeaderValue::from_static("keep-alive, x-trace"),
        );
        h.insert("x-trace", HeaderValue::from_static("1"));
        h.insert("keep-alive", HeaderValue::from_static("timeout=5"));
        h.insert(
            header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        h.insert(
            header::PROXY_AUTHORIZATION,
            HeaderValue::from_static("Basic eA=="),
        );
        h.insert(
            "anthropic-beta",
            HeaderValue::from_static("oauth-2025-04-20"),
        );
        drop_hop_by_hop(&mut h);
        let left: Vec<&str> = h.keys().map(HeaderName::as_str).collect();
        assert_eq!(left, ["anthropic-beta"]);
    }

    /// What the stand-in for the Anthropic API was asked: each request's
    /// path, `Authorization` and body.
    type Heard = Arc<Mutex<Vec<(String, Option<String>, String)>>>;

    /// A stand-in for the Anthropic API on `localhost`, with a certificate of
    /// its own authority: what it was asked, and the TLS to reach it.
    struct Api {
        origin: String,
        tls: rustls::ClientConfig,
        heard: Heard,
    }

    async fn api() -> Result<Api> {
        let key = KeyPair::generate()?;
        let cert = CertificateParams::new(vec!["localhost".to_owned()])?.self_signed(&key)?;
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert.der().clone())?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tls = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots)
            .with_no_client_auth();
        let mut server = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der().clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
            )?;
        server.alpn_protocols = vec![b"http/1.1".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("https://localhost:{}", listener.local_addr()?.port());
        let heard: Heard = Arc::default();
        let log = heard.clone();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let log = log.clone();
                tokio::spawn(async move {
                    let Ok(tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    let service = service_fn(move |req: Request<Incoming>| {
                        let log = log.clone();
                        async move {
                            let path = req.uri().path().to_owned();
                            let auth = req
                                .headers()
                                .get(header::AUTHORIZATION)
                                .and_then(|v| v.to_str().ok())
                                .map(str::to_owned);
                            let body = req
                                .into_body()
                                .collect()
                                .await
                                .map(|b| b.to_bytes())
                                .unwrap_or_default();
                            log.lock().unwrap_or_else(PoisonError::into_inner).push((
                                path,
                                auth,
                                String::from_utf8_lossy(&body).into_owned(),
                            ));
                            let events = "event: message_start\ndata: {}\n\nevent: message_stop\ndata: {}\n\n";
                            let res = Response::builder()
                                .header(header::CONTENT_TYPE, "text/event-stream")
                                .body(Full::new(Bytes::from(events)));
                            Ok::<_, Infallible>(res.unwrap_or_default())
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(tls), service)
                        .await;
                });
            }
        });
        Ok(Api { origin, tls, heard })
    }

    /// TLS to the host, as Claude would: trusting `authority` alone, for
    /// the Anthropic API's name.
    async fn claude(
        io: tokio::io::DuplexStream,
        authority: &Authority,
    ) -> Result<tokio_rustls::client::TlsStream<tokio::io::DuplexStream>> {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from_pem_slice(authority.pem().as_bytes())?)?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots)
            .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let name = rustls::pki_types::ServerName::try_from(CLAUDE_API_HOST)?;
        Ok(tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(name, io)
            .await?)
    }

    /// Sends one request on `tls` and reads the whole answer.
    async fn ask(
        tls: &mut tokio_rustls::client::TlsStream<tokio::io::DuplexStream>,
        auth: Option<&str>,
        body: &str,
    ) -> Result<String> {
        let auth = auth
            .map(|a| format!("authorization: {a}\r\n"))
            .unwrap_or_default();
        let head = format!(
            "POST /v1/messages?beta=true HTTP/1.1\r\nhost: {CLAUDE_API_HOST}\r\n{auth}content-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        tls.write_all(head.as_bytes()).await?;
        let mut answer = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = tls.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            answer.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&answer);
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text[..end]
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().to_owned())
                    })
                    .and_then(|v| v.parse::<usize>().ok());
                let chunked_done = text[end..].contains("\r\n0\r\n\r\n");
                if length.is_some_and(|l| answer.len() >= end + 4 + l) || chunked_done {
                    break;
                }
            }
        }
        Ok(String::from_utf8_lossy(&answer).into_owned())
    }

    #[tokio::test]
    async fn claudes_requests_reach_the_api_with_the_token_in_place_of_the_stand_in() -> Result<()>
    {
        let dir = tempfile::tempdir()?;
        let authority = Authority::load_or_make(&dir.path().join(FILE))?;
        let api = api().await?;
        let token = Token::default();
        token.set("sk-ant-oat01-the-real-one");
        let (sandbox, host) = tokio::io::duplex(64 * 1024);
        let served = tokio::spawn(serve(
            host,
            authority.server()?,
            token.clone(),
            Upstream::new(api.origin.clone(), api.tls.clone()),
        ));
        let mut tls = claude(sandbox, &authority).await?;
        let answer = ask(
            &mut tls,
            Some(&format!("Bearer {CLAUDE_STAND_IN}")),
            "{\"stream\":true}",
        )
        .await?;
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        assert!(answer.contains("event: message_start"), "{answer}");
        // A request without a token, and one with another, go on as they are,
        // on the same connection.
        ask(&mut tls, None, "{}").await?;
        ask(&mut tls, Some("Bearer sk-ant-oat01-somebody-elses"), "{}").await?;
        // A token the session got since replaces the one it had.
        token.set("sk-ant-oat01-a-newer-one");
        ask(&mut tls, Some(&format!("Bearer {CLAUDE_STAND_IN}")), "{}").await?;
        drop(tls);
        let heard = api
            .heard
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let auths: Vec<Option<&str>> = heard.iter().map(|(_, a, _)| a.as_deref()).collect();
        assert_eq!(
            auths,
            [
                Some("Bearer sk-ant-oat01-the-real-one"),
                None,
                Some("Bearer sk-ant-oat01-somebody-elses"),
                Some("Bearer sk-ant-oat01-a-newer-one"),
            ]
        );
        assert_eq!(heard[0].0, "/v1/messages");
        assert_eq!(heard[0].2, "{\"stream\":true}");
        let _ = tokio::time::timeout(Duration::from_secs(5), served).await;
        Ok(())
    }

    #[tokio::test]
    async fn a_request_waits_for_a_token_the_host_doesnt_know_yet() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let authority = Authority::load_or_make(&dir.path().join(FILE))?;
        let api = api().await?;
        let token = Token::default();
        let (sandbox, host) = tokio::io::duplex(64 * 1024);
        tokio::spawn(serve(
            host,
            authority.server()?,
            token.clone(),
            Upstream::new(api.origin.clone(), api.tls.clone()),
        ));
        let mut tls = claude(sandbox, &authority).await?;
        let later = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            later.set("sk-ant-oat01-fetched-again");
        });
        let answer = ask(&mut tls, Some(&format!("Bearer {CLAUDE_STAND_IN}")), "{}").await?;
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        let heard = api
            .heard
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        assert_eq!(
            heard[0].1.as_deref(),
            Some("Bearer sk-ant-oat01-fetched-again")
        );
        Ok(())
    }

    #[tokio::test]
    async fn the_api_out_of_reach_is_said_as_the_api_would() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let authority = Authority::load_or_make(&dir.path().join(FILE))?;
        let api = api().await?;
        let token = Token::default();
        token.set("sk-ant-oat01-the-real-one");
        let (sandbox, host) = tokio::io::duplex(64 * 1024);
        // Nothing listens there.
        tokio::spawn(serve(
            host,
            authority.server()?,
            token,
            Upstream::new("https://localhost:9".into(), api.tls.clone()),
        ));
        let mut tls = claude(sandbox, &authority).await?;
        let answer = ask(&mut tls, Some(&format!("Bearer {CLAUDE_STAND_IN}")), "{}").await?;
        assert!(answer.starts_with("HTTP/1.1 502"), "{answer}");
        assert!(
            answer.contains("couldn't reach the Anthropic API"),
            "{answer}"
        );
        assert!(!answer.contains("the-real-one"), "{answer}");
        Ok(())
    }
}
