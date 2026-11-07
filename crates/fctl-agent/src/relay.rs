//! Claude's HTTPS proxy, where the host keeps Claude's token
//! ([`fctl_proto::agent::AgentConfig::claude_relay`]).
//!
//! Claude, and everything it runs, sends its HTTPS through this proxy
//! (`HTTPS_PROXY`), as `CONNECT host:port`. A connection to the Anthropic API
//! ([`CLAUDE_API_HOST`]) waits for the host: fctl-agent asks the host for it
//! ([`fctl_proto::agent::AgentToHost::ClaudeConnection`]), the host opens a
//! channel to the relay socket, as it attaches to fctl-agent, and names the
//! connection on it, and the two are joined. The host then answers Claude's
//! TLS as the Anthropic API and puts the token in. Every other connection
//! goes out as it would have without the proxy: straight, or through the
//! proxy the session had before.

use anyhow::{Context, Result, bail};
use base64::Engine;
use fctl_proto::agent::CLAUDE_API_HOST;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};

/// Longest request head the proxy reads.
const MAX_HEAD: usize = 16 * 1024;

/// How long a client has to send its request, and the host to open the
/// channel for a connection to the Anthropic API.
const WAIT: Duration = Duration::from_secs(30);

/// The relay socket, beside the host's: `agent.sock` → `agent.relay.sock`.
pub fn socket_path(host_socket: &std::path::Path) -> std::path::PathBuf {
    host_socket.with_extension("relay.sock")
}

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<UnixStream>>>>;

/// The proxy, while the session runs.
pub struct Relay {
    port: u16,
    pending: Pending,
}

impl Relay {
    /// Listens on `127.0.0.1:port`, or on another port when that one is
    /// taken (or `port` is 0), and takes the host's channels on `socket`.
    /// The connections to the Anthropic API that need one go to `ask`, by
    /// id; the others go out through `upstream` when the session had a
    /// proxy, except those `upstream` leaves alone.
    pub fn start(
        port: u16,
        socket: UnixListener,
        upstream: Option<Upstream>,
        ask: mpsc::UnboundedSender<String>,
    ) -> Result<Self> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", port))
            .or_else(|_| std::net::TcpListener::bind(("127.0.0.1", 0)))
            .context("listening for Claude's connections")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let listener = TcpListener::from_std(listener)?;
        let pending: Pending = Arc::default();
        let upstream = Arc::new(upstream);
        let ids = Arc::new(AtomicU64::new(0));
        let waiting = pending.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let (pending, upstream, ask, ids) =
                    (waiting.clone(), upstream.clone(), ask.clone(), ids.clone());
                tokio::spawn(async move {
                    if let Err(e) = client(stream, &pending, &upstream, &ask, &ids).await {
                        tracing::debug!("a connection through Claude's proxy: {e:#}");
                    }
                });
            }
        });
        tokio::spawn(channels(socket, pending.clone()));
        Ok(Self { port, pending })
    }

    /// The port it listens on, on `127.0.0.1`.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// No host will open the channel for `id`: its client hears so at once.
    pub fn refuse(&self, id: &str) {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
    }
}

/// Takes the host's channels: each names the connection it is for on its
/// first line.
async fn channels(socket: UnixListener, pending: Pending) {
    loop {
        let Ok((mut stream, _)) = socket.accept().await else {
            continue;
        };
        let pending = pending.clone();
        tokio::spawn(async move {
            let id = match tokio::time::timeout(WAIT, line(&mut stream)).await {
                Ok(Ok(id)) => id,
                _ => return,
            };
            let waiter = pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
            match waiter {
                Some(tx) => {
                    let _ = tx.send(stream);
                }
                None => {
                    tracing::debug!("the host opened a channel for {id}, which no longer waits")
                }
            }
        });
    }
}

/// One line, read a byte at a time so that nothing after it is taken.
async fn line(stream: &mut UnixStream) -> Result<String> {
    let mut line = Vec::new();
    loop {
        let byte = stream.read_u8().await?;
        if byte == b'\n' {
            return Ok(String::from_utf8(line)?);
        }
        line.push(byte);
        if line.len() > 128 {
            bail!("the channel's first line is too long");
        }
    }
}

/// What a client asks the proxy for.
#[derive(Debug, PartialEq, Eq)]
enum Ask {
    /// `CONNECT host:port`: a tunnel.
    Connect(String, u16),
    /// A plain HTTP request for an absolute URL: its host and port, and the
    /// request with the URL's path alone.
    Plain(String, u16, Vec<u8>),
}

async fn client(
    mut stream: TcpStream,
    pending: &Pending,
    upstream: &Option<Upstream>,
    ask: &mpsc::UnboundedSender<String>,
    ids: &AtomicU64,
) -> Result<()> {
    let (head, rest) = tokio::time::timeout(WAIT, read_head(&mut stream))
        .await
        .context("the client didn't send its request")??;
    let request = match parse(&head) {
        Ok(r) => r,
        Err(e) => {
            answer(&mut stream, "400 Bad Request", &format!("{e:#}")).await;
            return Ok(());
        }
    };
    match request {
        Ask::Connect(host, port) if host.eq_ignore_ascii_case(CLAUDE_API_HOST) && port == 443 => {
            let id = format!("c{}", ids.fetch_add(1, Ordering::Relaxed) + 1);
            let (tx, rx) = oneshot::channel();
            pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(id.clone(), tx);
            let _ = ask.send(id.clone());
            let channel = match tokio::time::timeout(WAIT, rx).await {
                Ok(Ok(channel)) => channel,
                _ => {
                    pending
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&id);
                    answer(
                        &mut stream,
                        "502 Bad Gateway",
                        "The runner's host didn't open the connection to the Anthropic API: it keeps the Claude token, and may be restarting",
                    )
                    .await;
                    return Ok(());
                }
            };
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await?;
            join(stream, channel, &rest).await
        }
        Ask::Connect(host, port) => {
            let out = match connect(&host, port, upstream).await {
                Ok(out) => out,
                Err(e) => {
                    answer(&mut stream, "502 Bad Gateway", &format!("{e:#}")).await;
                    return Ok(());
                }
            };
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await?;
            join(stream, out, &rest).await
        }
        Ask::Plain(host, port, request) => {
            // Through the session's proxy, as it was; else to the server,
            // which then closes the connection after its answer, since the
            // client may send the next request on it for another server.
            let (mut out, first) = match upstream.as_ref().filter(|u| u.takes(&host)) {
                Some(u) => (u.dial().await?, u.authorize(&head)),
                None => (
                    tokio::time::timeout(WAIT, TcpStream::connect((host.as_str(), port)))
                        .await
                        .context("connecting")??,
                    request,
                ),
            };
            out.write_all(&first).await?;
            join(stream, out, &rest).await
        }
    }
}

/// Joins two streams until both sides are done, `rest` (what the client
/// sent after its request) going first.
async fn join<A, B>(mut a: A, mut b: B, rest: &[u8]) -> Result<()>
where
    A: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    B: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if !rest.is_empty() {
        b.write_all(rest).await?;
    }
    tokio::io::copy_bidirectional(&mut a, &mut b).await?;
    Ok(())
}

/// Says why the proxy can't do what the client asked, and closes.
async fn answer(stream: &mut TcpStream, status: &str, why: &str) {
    let text = format!(
        "HTTP/1.1 {status}\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{why}",
        why.len()
    );
    let _ = stream.write_all(text.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Reads the request head, through its blank line: the head, and what
/// came after it.
async fn read_head(stream: &mut TcpStream) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            bail!("the client closed the connection");
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(end + 4);
            return Ok((buf, rest));
        }
        if buf.len() > MAX_HEAD {
            bail!("the request head is longer than {MAX_HEAD} bytes");
        }
    }
}

/// What the request head asks for.
fn parse(head: &[u8]) -> Result<Ask> {
    let text = std::str::from_utf8(head).context("the request isn't text")?;
    let first = text.lines().next().unwrap_or_default();
    let mut words = first.split_whitespace();
    let (Some(method), Some(target), Some(version)) = (words.next(), words.next(), words.next())
    else {
        bail!("that isn't an HTTP request");
    };
    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = host_port(target, None)?;
        return Ok(Ask::Connect(host, port));
    }
    let Some(url) = target.strip_prefix("http://") else {
        bail!("the proxy takes CONNECT, and plain HTTP requests for http:// addresses");
    };
    let (authority, path) = match url.find('/') {
        Some(i) => (&url[..i], &url[i..]),
        None => (url, "/"),
    };
    let (host, port) = host_port(authority, Some(80))?;
    let mut request = format!("{method} {path} {version}\r\n");
    for line in text.lines().skip(1).filter(|l| !l.is_empty()) {
        let name = line.split(':').next().unwrap_or_default().trim();
        if [
            "connection",
            "proxy-connection",
            "proxy-authorization",
            "keep-alive",
        ]
        .iter()
        .any(|n| name.eq_ignore_ascii_case(n))
        {
            continue;
        }
        request.push_str(line);
        request.push_str("\r\n");
    }
    request.push_str("connection: close\r\n\r\n");
    Ok(Ask::Plain(host, port, request.into_bytes()))
}

/// `host:port`, `[v6]:port`, or `host` alone with `default`.
fn host_port(text: &str, default: Option<u16>) -> Result<(String, u16)> {
    let (host, port) = match text.rsplit_once(':') {
        Some((host, port)) if !port.contains(']') => (host, Some(port)),
        _ => (text, None),
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        bail!("no host in {text:?}");
    }
    let port = match port {
        Some(p) => p.parse().with_context(|| format!("no port in {text:?}"))?,
        None => default.with_context(|| format!("no port in {text:?}"))?,
    };
    Ok((host.to_owned(), port))
}

/// Opens a tunnel to `host:port`: through `upstream` unless it leaves the
/// host alone, else straight.
async fn connect(host: &str, port: u16, upstream: &Option<Upstream>) -> Result<TcpStream> {
    match upstream.as_ref().filter(|u| u.takes(host)) {
        Some(u) => u.tunnel(host, port).await,
        None => tokio::time::timeout(WAIT, TcpStream::connect((host, port)))
            .await
            .with_context(|| format!("connecting to {host}:{port}"))?
            .with_context(|| format!("connecting to {host}:{port}")),
    }
}

/// The HTTP proxy the session had before (`https_proxy`), which what Claude
/// runs keeps going through, except where `no_proxy` said not to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upstream {
    host: String,
    port: u16,
    /// `Basic …`, from the address's user and password.
    authorization: Option<String>,
    /// Hosts it leaves alone.
    bypass: Vec<String>,
}

impl Upstream {
    /// The proxy `url` names (`http://user:password@host:port`), leaving
    /// alone the hosts `no_proxy` lists. `None` for none, or one that isn't
    /// a plain HTTP proxy.
    pub fn parse(url: &str, no_proxy: Option<&str>) -> Option<Self> {
        let url = url.trim();
        let rest = url
            .strip_prefix("http://")
            .or_else(|| (!url.contains("://")).then_some(url))?;
        let authority = rest.split('/').next().unwrap_or_default();
        let (userinfo, hostport) = match authority.rsplit_once('@') {
            Some((u, h)) => (Some(u), h),
            None => (None, authority),
        };
        let (host, port) = host_port(hostport, Some(80)).ok()?;
        let authorization = userinfo.map(|u| {
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(percent_decode(u))
            )
        });
        let bypass = no_proxy
            .unwrap_or_default()
            .split([',', ' '])
            .map(|h| h.trim().trim_start_matches('*').to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect();
        Some(Self {
            host,
            port,
            authorization,
            bypass,
        })
    }

    /// Whether connections to `host` go through it.
    fn takes(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        !self.bypass.iter().any(|b| {
            let bare = b.trim_start_matches('.');
            host == bare || host.ends_with(&format!(".{bare}"))
        })
    }

    async fn dial(&self) -> Result<TcpStream> {
        tokio::time::timeout(WAIT, TcpStream::connect((self.host.as_str(), self.port)))
            .await
            .with_context(|| format!("connecting to the proxy {}:{}", self.host, self.port))?
            .with_context(|| format!("connecting to the proxy {}:{}", self.host, self.port))
    }

    /// A plain request's `head`, with its credentials for it.
    fn authorize(&self, head: &[u8]) -> Vec<u8> {
        let Some(a) = &self.authorization else {
            return head.to_vec();
        };
        let mut out = head.strip_suffix(b"\r\n").unwrap_or(head).to_vec();
        out.extend_from_slice(format!("proxy-authorization: {a}\r\n\r\n").as_bytes());
        out
    }

    /// A tunnel to `host:port` through it.
    async fn tunnel(&self, host: &str, port: u16) -> Result<TcpStream> {
        let mut out = self.dial().await?;
        let target = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let auth = self
            .authorization
            .as_ref()
            .map(|a| format!("proxy-authorization: {a}\r\n"))
            .unwrap_or_default();
        out.write_all(
            format!("CONNECT {target} HTTP/1.1\r\nhost: {target}\r\n{auth}\r\n").as_bytes(),
        )
        .await?;
        // Its answer, a byte at a time: what follows it is the server's.
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(out.read_u8().await?);
            if head.len() > MAX_HEAD {
                bail!("the proxy's answer is too long");
            }
        }
        let text = String::from_utf8_lossy(&head);
        let status = text.lines().next().unwrap_or_default();
        if status.split_whitespace().nth(1) != Some("200") {
            bail!("the proxy {}:{} answered {status}", self.host, self.port);
        }
        Ok(out)
    }
}

/// `%XX` escapes decoded, as user names and passwords in addresses have them.
fn percent_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3])
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// What `no_proxy` should say for Claude: loopback, and what the session
/// said, but never anything that would take the Anthropic API off the
/// proxy, where the host puts the token in.
pub fn no_proxy(session: Option<&str>) -> String {
    let api = CLAUDE_API_HOST.to_ascii_lowercase();
    let mut hosts = vec![
        "localhost".to_owned(),
        "127.0.0.1".to_owned(),
        "::1".to_owned(),
    ];
    for h in session.unwrap_or_default().split([',', ' ']) {
        let h = h.trim();
        let bare = h
            .trim_start_matches('*')
            .trim_start_matches('.')
            .to_ascii_lowercase();
        let covers_api =
            h == "*" || bare.is_empty() || api == bare || api.ends_with(&format!(".{bare}"));
        if !covers_api && !hosts.iter().any(|x| x.eq_ignore_ascii_case(h)) {
            hosts.push(h.to_owned());
        }
    }
    hosts.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_clients_ask_for() -> Result<()> {
        assert_eq!(
            parse(
                b"CONNECT api.anthropic.com:443 HTTP/1.1\r\nHost: api.anthropic.com:443\r\n\r\n"
            )?,
            Ask::Connect("api.anthropic.com".into(), 443)
        );
        assert_eq!(
            parse(b"CONNECT [::1]:8443 HTTP/1.1\r\n\r\n")?,
            Ask::Connect("::1".into(), 8443)
        );
        let Ask::Plain(host, port, request) = parse(
            b"GET http://example.com:8080/a?b=c HTTP/1.1\r\nHost: example.com:8080\r\nProxy-Connection: keep-alive\r\nAccept: */*\r\n\r\n",
        )?
        else {
            bail!("a plain request");
        };
        assert_eq!((host.as_str(), port), ("example.com", 8080));
        assert_eq!(
            String::from_utf8(request)?,
            "GET /a?b=c HTTP/1.1\r\nHost: example.com:8080\r\nAccept: */*\r\nconnection: close\r\n\r\n"
        );
        assert!(parse(b"GET /relative HTTP/1.1\r\n\r\n").is_err());
        assert!(parse(b"CONNECT api.anthropic.com HTTP/1.1\r\n\r\n").is_err());
        Ok(())
    }

    #[test]
    fn the_anthropic_api_never_leaves_the_proxy() {
        assert_eq!(no_proxy(None), "localhost,127.0.0.1,::1");
        assert_eq!(
            no_proxy(Some(
                "*.internal, .anthropic.com,api.anthropic.com,*,anthropic.com,10.0.0.1"
            )),
            "localhost,127.0.0.1,::1,*.internal,10.0.0.1"
        );
    }

    #[test]
    fn the_sessions_proxy_is_kept_for_what_claude_runs() -> Result<()> {
        let u = Upstream::parse(
            "http://ada:p%40ss@proxy.corp:3128/",
            Some("internal.corp, .local"),
        );
        let Some(u) = u else {
            bail!("a proxy");
        };
        assert_eq!((u.host.as_str(), u.port), ("proxy.corp", 3128));
        assert_eq!(
            u.authorization.as_deref(),
            Some(
                format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode("ada:p@ss")
                )
                .as_str()
            )
        );
        assert!(u.takes("github.com"));
        assert!(!u.takes("internal.corp"));
        assert!(!u.takes("git.internal.corp"));
        assert!(!u.takes("printer.local"));
        assert_eq!(
            Upstream::parse("proxy:8080", None).map(|u| u.port),
            Some(8080)
        );
        // Not a plain HTTP proxy.
        assert_eq!(Upstream::parse("https://proxy:8443", None), None);
        assert_eq!(Upstream::parse("socks5://proxy:1080", None), None);
        // A plain request gets its credentials for it.
        assert_eq!(
            String::from_utf8(u.authorize(b"GET http://a/ HTTP/1.1\r\nHost: a\r\n\r\n"))?,
            format!(
                "GET http://a/ HTTP/1.1\r\nHost: a\r\nproxy-authorization: Basic {}\r\n\r\n",
                base64::engine::general_purpose::STANDARD.encode("ada:p@ss")
            )
        );
        Ok(())
    }

    /// A client that asks the proxy for `target`.
    async fn ask(port: u16, target: &str) -> Result<(TcpStream, String)> {
        let mut c = TcpStream::connect(("127.0.0.1", port)).await?;
        c.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
            .await?;
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(c.read_u8().await?);
        }
        Ok((c, String::from_utf8(head)?))
    }

    #[tokio::test]
    async fn the_anthropic_api_goes_to_the_host_and_the_rest_straight_out() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let socket_path = dir.path().join("agent.relay.sock");
        let socket = UnixListener::bind(&socket_path)?;
        let (ask_tx, mut asked) = mpsc::unbounded_channel();
        let relay = Relay::start(0, socket, None, ask_tx)?;

        // A connection to the Anthropic API waits for the host's channel.
        let port = relay.port();
        let claude = tokio::spawn(async move { ask(port, "api.anthropic.com:443").await });
        let id = tokio::time::timeout(WAIT, asked.recv())
            .await?
            .context("the host was asked")?;
        let mut channel = UnixStream::connect(&socket_path).await?;
        channel.write_all(format!("{id}\n").as_bytes()).await?;
        let (mut c, head) = claude.await??;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        c.write_all(b"client hello").await?;
        let mut got = [0u8; 12];
        channel.read_exact(&mut got).await?;
        assert_eq!(&got, b"client hello");
        channel.write_all(b"server hello").await?;
        c.read_exact(&mut got).await?;
        assert_eq!(&got, b"server hello");

        // One the host won't open hears so at once.
        let claude = tokio::spawn(async move { ask(port, "api.anthropic.com:443").await });
        let id = tokio::time::timeout(WAIT, asked.recv())
            .await?
            .context("the host was asked")?;
        relay.refuse(&id);
        let (_, head) = claude.await??;
        assert!(head.starts_with("HTTP/1.1 502"), "{head}");

        // Any other host is reached from the sandbox, without the host.
        let server = TcpListener::bind("127.0.0.1:0").await?;
        let target = format!("localhost:{}", server.local_addr()?.port());
        let (mut c, head) = ask(port, &target).await?;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        let (mut s, _) = server.accept().await?;
        c.write_all(b"ping").await?;
        let mut got = [0u8; 4];
        s.read_exact(&mut got).await?;
        assert_eq!(&got, b"ping");
        assert!(asked.try_recv().is_err());
        Ok(())
    }

    #[tokio::test]
    async fn other_hosts_go_through_the_sessions_own_proxy() -> Result<()> {
        // The session's proxy: it says yes to CONNECT, then echoes.
        let proxy = TcpListener::bind("127.0.0.1:0").await?;
        let proxy_port = proxy.local_addr()?.port();
        let (heard_tx, mut heard) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = proxy.accept().await {
                let heard_tx = heard_tx.clone();
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    while !head.ends_with(b"\r\n\r\n") {
                        let Ok(b) = s.read_u8().await else { return };
                        head.push(b);
                    }
                    let _ = heard_tx.send(String::from_utf8_lossy(&head).into_owned());
                    let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await;
                    let mut buf = [0u8; 4];
                    if s.read_exact(&mut buf).await.is_ok() {
                        let _ = s.write_all(&buf).await;
                    }
                });
            }
        });
        let dir = tempfile::tempdir()?;
        let socket = UnixListener::bind(dir.path().join("agent.relay.sock"))?;
        let upstream = Upstream::parse(&format!("http://ada:pw@127.0.0.1:{proxy_port}"), None);
        let (ask_tx, _asked) = mpsc::unbounded_channel();
        let relay = Relay::start(0, socket, upstream, ask_tx)?;
        let (mut c, head) = ask(relay.port(), "github.com:443").await?;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        let said = heard.recv().await.context("the proxy was asked")?;
        assert!(
            said.starts_with("CONNECT github.com:443 HTTP/1.1\r\n"),
            "{said}"
        );
        assert!(
            said.contains("proxy-authorization: Basic YWRhOnB3\r\n"),
            "{said}"
        );
        c.write_all(b"echo").await?;
        let mut got = [0u8; 4];
        c.read_exact(&mut got).await?;
        assert_eq!(&got, b"echo");
        Ok(())
    }
}
