//! Web Push (RFC 8030): encrypting a message for a browser (RFC 8291, in the
//! `aes128gcm` content coding of RFC 8188) and the VAPID token that says who
//! sends it (RFC 8292).
//!
//! The elliptic-curve steps run where the keys are (WebCrypto, in the
//! Worker). This module takes their results, the ECDH secret and the token's
//! signature, and does the rest.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// The size of the one record a message is encrypted into.
const RECORD_SIZE: u32 = 4096;
/// Salt (16 bytes), record size (4), key id length (1) and key id (65).
pub const HEADER_LEN: usize = 86;
/// The longest message that fits the 4096 bytes every push service accepts:
/// what the header, the AES-GCM tag (16) and the padding delimiter (1) leave.
pub const MAX_PLAINTEXT: usize = 4096 - HEADER_LEN - 16 - 1;
/// An uncompressed P-256 public key: `0x04 ‖ x ‖ y`.
pub const PUBLIC_KEY_LEN: usize = 65;
/// A browser's authentication secret.
pub const AUTH_LEN: usize = 16;
/// How long a VAPID token is valid (RFC 8292 allows at most 24 hours).
pub const TOKEN_TTL_SECS: i64 = 12 * 3600;

/// Why a message could not be encrypted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Longer than [`MAX_PLAINTEXT`].
    TooLong,
    /// A key or secret has the wrong length.
    Key,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooLong => "the push message is too long",
            Self::Key => "a push key has the wrong length",
        })
    }
}

/// A browser's subscription keys: what its messages are encrypted for.
pub struct Subscriber<'a> {
    /// `p256dh`: its P-256 public key, uncompressed.
    pub public_key: &'a [u8],
    /// `auth`: its authentication secret.
    pub auth: &'a [u8],
}

/// The sender's side of one message: the public key of a key pair made for
/// it, the ECDH secret that key pair shares with the browser, and a random
/// salt.
pub struct Sender<'a> {
    pub public_key: &'a [u8],
    pub ecdh_secret: &'a [u8],
    pub salt: &'a [u8; 16],
}

type HmacSha256 = Hmac<Sha256>;

fn hmac(key: &[u8], parts: &[&[u8]]) -> Result<[u8; 32], Error> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| Error::Key)?;
    for part in parts {
        mac.update(part);
    }
    Ok(mac.finalize().into_bytes().into())
}

/// The body of a push message: `plaintext` encrypted for `to` in a single
/// `aes128gcm` record (RFC 8291 section 3.4).
pub fn encrypt(plaintext: &[u8], to: &Subscriber, from: &Sender) -> Result<Vec<u8>, Error> {
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(Error::TooLong);
    }
    if to.public_key.len() != PUBLIC_KEY_LEN
        || from.public_key.len() != PUBLIC_KEY_LEN
        || to.auth.len() != AUTH_LEN
        || from.ecdh_secret.len() != 32
    {
        return Err(Error::Key);
    }
    // The ECDH secret combined with the browser's authentication secret
    // (section 3.3), then the content encryption key and nonce (RFC 8188
    // section 2.2). Each HKDF expansion is a single HMAC block.
    let prk_key = hmac(to.auth, &[from.ecdh_secret])?;
    let ikm = hmac(
        &prk_key,
        &[b"WebPush: info\0", to.public_key, from.public_key, &[1]],
    )?;
    let prk = hmac(from.salt, &[&ikm])?;
    let cek = hmac(&prk, &[b"Content-Encoding: aes128gcm\0", &[1]])?;
    let nonce = hmac(&prk, &[b"Content-Encoding: nonce\0", &[1]])?;
    let cipher =
        Aes128Gcm::new_from_slice(cek.get(..16).ok_or(Error::Key)?).map_err(|_| Error::Key)?;
    let nonce: [u8; 12] = nonce
        .get(..12)
        .and_then(|n| n.try_into().ok())
        .ok_or(Error::Key)?;
    // The last record ends with the padding delimiter 2, and this is the
    // only one.
    let mut record = Vec::with_capacity(plaintext.len() + 1);
    record.extend_from_slice(plaintext);
    record.push(2);
    let ciphertext = cipher
        .encrypt(&Nonce::from(nonce), record.as_slice())
        .map_err(|_| Error::Key)?;
    let mut body = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    body.extend_from_slice(from.salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(PUBLIC_KEY_LEN as u8);
    body.extend_from_slice(from.public_key);
    body.extend_from_slice(&ciphertext);
    Ok(body)
}

/// The origin of a push endpoint (`https://fcm.googleapis.com`): the
/// audience of the VAPID token sent to it.
pub fn audience(endpoint: &str) -> Option<String> {
    let (scheme, rest) = endpoint.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    (matches!(scheme, "https" | "http") && !host.is_empty() && !host.contains('@'))
        .then(|| format!("{scheme}://{host}"))
}

/// The push service of an endpoint, as people see it: its host name.
pub fn service(endpoint: &str) -> String {
    let origin = audience(endpoint).unwrap_or_default();
    let host = origin.split_once("://").map_or("", |(_, h)| h);
    match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => name.to_owned(),
        _ => host.to_owned(),
    }
}

/// What the sender signs for a VAPID token (RFC 8292 section 2): the JWT's
/// header and claims, base64url-encoded and joined by a dot.
pub fn token_input(audience: &str, subject: &str, now_ms: i64) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = serde_json::json!({
        "aud": audience,
        "exp": now_ms / 1000 + TOKEN_TTL_SECS,
        "sub": subject,
    });
    format!("{header}.{}", URL_SAFE_NO_PAD.encode(claims.to_string()))
}

/// The `Authorization` header of a push request: the token with its ES256
/// signature (`r ‖ s`), and the public key that verifies it.
pub fn authorization(token_input: &str, signature: &[u8], public_key: &[u8]) -> String {
    format!(
        "vapid t={token_input}.{}, k={}",
        URL_SAFE_NO_PAD.encode(signature),
        URL_SAFE_NO_PAD.encode(public_key)
    )
}

/// What a push service's answer says about a subscription.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// The service took the message.
    Accepted,
    /// The subscription expired or the browser unsubscribed: forget it.
    Gone,
    /// Anything else; the subscription may work next time.
    Failed,
}

/// Reads a push service's HTTP status (RFC 8030 sections 5 and 7.3).
pub fn delivery(status: u16) -> Delivery {
    match status {
        200..=299 => Delivery::Accepted,
        404 | 410 => Delivery::Gone,
        _ => Delivery::Failed,
    }
}

/// Bytes as base64url without padding, the way browsers give push keys.
pub fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// base64url, with or without padding.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(text.trim().trim_end_matches('='))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &str) -> Vec<u8> {
        let joined: String = s.split_whitespace().collect();
        decode(&joined).unwrap_or_default()
    }

    /// RFC 8291 section 5 and appendix A.
    #[test]
    fn encrypts_the_rfc_example() -> Result<(), Error> {
        let plaintext = b"When I grow up, I want to be a watermelon";
        let ua_public = b64(
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcx aOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
        );
        let as_public = b64(
            "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIg Dll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8",
        );
        let auth = b64("BTBZMqHH6r4Tts7J_aSIgg");
        let salt: [u8; 16] = b64("DGv6ra1nlYgDCS1FRnbzlw")
            .try_into()
            .map_err(|_| Error::Key)?;
        let ecdh_secret = b64("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs");
        let body = encrypt(
            plaintext,
            &Subscriber {
                public_key: &ua_public,
                auth: &auth,
            },
            &Sender {
                public_key: &as_public,
                ecdh_secret: &ecdh_secret,
                salt: &salt,
            },
        )?;
        let header = b64(
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z 9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml
             mlMoZIIgDll6e3vCYLocInmYWAmS6Tlz AC8wEqKK6PBru3jl7A8",
        );
        let ciphertext = b64(
            "8pfeW0KbunFT06SuDKoJH9Ql87S1QUrd irN6GcG7sFz1y1sqLgVi1VhjVkHsUoEs
             bI_0LpXMuGvnzQ",
        );
        assert_eq!(header.len(), HEADER_LEN);
        assert_eq!(body.get(..HEADER_LEN), Some(header.as_slice()));
        assert_eq!(body.get(HEADER_LEN..), Some(ciphertext.as_slice()));
        assert_eq!(
            body,
            b64(
                "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml
                 mlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPT
                 pK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
            )
        );
        Ok(())
    }

    #[test]
    fn refuses_what_does_not_fit_or_does_not_match() {
        let key = [4u8; PUBLIC_KEY_LEN];
        let to = Subscriber {
            public_key: &key,
            auth: &[0; AUTH_LEN],
        };
        let from = Sender {
            public_key: &key,
            ecdh_secret: &[0; 32],
            salt: &[0; 16],
        };
        let longest = encrypt(&[b'x'; MAX_PLAINTEXT], &to, &from).map(|b| b.len());
        assert_eq!(longest, Ok(4096));
        assert_eq!(
            encrypt(&[b'x'; MAX_PLAINTEXT + 1], &to, &from),
            Err(Error::TooLong)
        );
        let short = Subscriber {
            public_key: &key,
            auth: &[0; 8],
        };
        assert_eq!(encrypt(b"hi", &short, &from), Err(Error::Key));
    }

    #[test]
    fn vapid_tokens() {
        assert_eq!(
            audience("https://fcm.googleapis.com/fcm/send/abc:def").as_deref(),
            Some("https://fcm.googleapis.com")
        );
        assert_eq!(
            audience("http://127.0.0.1:9000/push?x=1").as_deref(),
            Some("http://127.0.0.1:9000")
        );
        assert_eq!(audience("ftp://example.com/x"), None);
        assert_eq!(audience("https://user@example.com/x"), None);
        assert_eq!(
            service("https://updates.push.services.mozilla.com/wpush/v2/gAAA"),
            "updates.push.services.mozilla.com"
        );
        assert_eq!(service("http://127.0.0.1:9000/push"), "127.0.0.1");

        let input = token_input(
            "https://push.example.net",
            "mailto:push@example.com",
            1_453_523_768_000,
        );
        let (header, claims) = input.split_once('.').unwrap_or_default();
        assert_eq!(b64(header), br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims: serde_json::Value = serde_json::from_slice(&b64(claims)).unwrap_or_default();
        assert_eq!(
            claims,
            serde_json::json!({
                "aud": "https://push.example.net",
                "exp": 1_453_523_768 + TOKEN_TTL_SECS,
                "sub": "mailto:push@example.com",
            })
        );
        assert_eq!(
            authorization("h.c", &[1, 2, 3], &[4, 5, 6]),
            "vapid t=h.c.AQID, k=BAUG"
        );
    }

    #[test]
    fn push_service_answers() {
        assert_eq!(delivery(201), Delivery::Accepted);
        assert_eq!(delivery(410), Delivery::Gone);
        assert_eq!(delivery(404), Delivery::Gone);
        assert_eq!(delivery(413), Delivery::Failed);
        assert_eq!(delivery(429), Delivery::Failed);
    }
}
