//! WebSocket connections to the control plane.

use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// `https://x/y` → `wss://x/y` (and http → ws).
pub fn ws_url(http_url: &str) -> String {
    if let Some(rest) = http_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = http_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        http_url.to_owned()
    }
}

/// Opens a WebSocket with a bearer token.
pub async fn connect(url: &str, token: Option<&str>) -> anyhow::Result<Socket> {
    let mut req = ws_url(url).into_client_request()?;
    let headers = req.headers_mut();
    headers.insert("User-Agent", HeaderValue::from_str(&crate::user_agent())?);
    if let Some(t) = token {
        headers.insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {t}"))?,
        );
    }
    let connector = Connector::Rustls(crate::tls_config());
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(64 << 20))
        .max_frame_size(Some(32 << 20));
    let fut =
        tokio_tungstenite::connect_async_tls_with_config(req, Some(config), false, Some(connector));
    let (socket, _) = tokio::time::timeout(Duration::from_secs(20), fut)
        .await
        .map_err(|_| anyhow::anyhow!("timed out connecting to {url}"))??;
    Ok(socket)
}

/// Sends a JSON text frame.
pub async fn send_json<T: serde::Serialize>(
    socket: &mut futures::stream::SplitSink<Socket, Message>,
    msg: &T,
) -> anyhow::Result<()> {
    socket
        .send(Message::text(serde_json::to_string(msg)?))
        .await?;
    Ok(())
}

/// The next text frame, skipping pings and pongs. `None` when closed.
pub async fn next_text(
    stream: &mut futures::stream::SplitStream<Socket>,
) -> Option<anyhow::Result<String>> {
    loop {
        match stream.next().await? {
            Ok(Message::Text(t)) => return Some(Ok(t.to_string())),
            Ok(Message::Binary(b)) => return Some(Ok(String::from_utf8_lossy(&b).into_owned())),
            Ok(Message::Close(_)) => return None,
            Ok(_) => continue,
            Err(e) => return Some(Err(e.into())),
        }
    }
}
