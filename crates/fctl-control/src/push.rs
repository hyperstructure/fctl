//! Sending Web Push notifications (RFC 8030) to people's browsers.
//!
//! The instance has one VAPID key pair (RFC 8292) that says the messages are
//! its own. It is made on first use and kept in D1's `settings` table, sealed
//! with the master key; browsers subscribe with its public key. Every
//! message is encrypted for its browser (RFC 8291) with a key pair made for
//! it. WebCrypto does the elliptic-curve work, `fctl_domain::webpush` the
//! rest.

use crate::config::Config;
use crate::crypto::{self, Keys};
use crate::ctx::now_ms;
use crate::db::{Val, batch, n, q, s};
use crate::error::{AppResult, internal};
use fctl_domain::random_bytes;
use fctl_domain::webpush::{self, Delivery, Sender, Subscriber};
use fctl_proto::api::{PushMessage, PushTestResult};
use js_sys::{Array, Object, Reflect, Uint8Array};
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use worker::{D1Database, Fetch, Headers, Method, Request, RequestInit, RequestRedirect};

/// How long a push service keeps a message for a browser that is offline.
const TTL_SECS: u32 = 12 * 3600;
/// Browsers one person can turn notifications on in.
pub const MAX_DEVICES: i64 = 20;
/// The row in `settings` that holds the VAPID key.
const VAPID_KEY: &str = "vapid";
/// Associated data of the sealed VAPID key.
const VAPID_AAD: &str = "settings:vapid";

/// The VAPID private key as JWK members (base64url P-256 coordinates).
#[derive(Serialize, Deserialize)]
struct StoredKey {
    d: String,
    x: String,
    y: String,
}

/// The instance's VAPID key pair, ready to sign.
pub struct Vapid {
    signing: web_sys::CryptoKey,
    /// The public key, an uncompressed P-256 point: what browsers subscribe with.
    pub public: Vec<u8>,
}

#[derive(Deserialize)]
struct SettingRow {
    value: String,
}

fn set(obj: &Object, key: &str, value: &JsValue) -> AppResult<()> {
    Reflect::set(obj, &JsValue::from_str(key), value)?;
    Ok(())
}

/// `{ name, namedCurve: "P-256" }` for ECDSA or ECDH.
fn p256(name: &str) -> AppResult<Object> {
    let algo = Object::new();
    set(&algo, "name", &name.into())?;
    set(&algo, "namedCurve", &"P-256".into())?;
    Ok(algo)
}

async fn stored(db: &D1Database) -> AppResult<Option<String>> {
    let row: Option<SettingRow> = q(
        db,
        "SELECT value FROM settings WHERE key = ?",
        &[s(VAPID_KEY)],
    )?
    .first()
    .await?;
    Ok(row.map(|r| r.value))
}

async fn generate() -> AppResult<StoredKey> {
    let subtle = crypto::subtle()?;
    let usages = Array::of2(&"sign".into(), &"verify".into());
    let promise = subtle.generate_key_with_object(&p256("ECDSA")?, true, &usages)?;
    // A CryptoKeyPair is a dictionary, not a class: nothing to check it against.
    let pair: web_sys::CryptoKeyPair = JsFuture::from(promise).await?.unchecked_into();
    let jwk = JsFuture::from(subtle.export_key("jwk", &pair.get_private_key())?).await?;
    let member = |name: &str| {
        Reflect::get(&jwk, &JsValue::from_str(name))
            .ok()
            .and_then(|v| v.as_string())
            .ok_or_else(|| internal(format!("the new VAPID key has no {name}")))
    };
    Ok(StoredKey {
        d: member("d")?,
        x: member("x")?,
        y: member("y")?,
    })
}

/// The instance's VAPID key pair, made and stored on first use.
pub async fn vapid(db: &D1Database, keys: &Keys) -> AppResult<Vapid> {
    let sealed = match stored(db).await? {
        Some(sealed) => sealed,
        None => {
            let sealed = keys.seal_json(VAPID_AAD, &generate().await?)?;
            // Two made at once: the first one stored wins.
            q(
                db,
                "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT (key) DO NOTHING",
                &[s(VAPID_KEY), s(&sealed)],
            )?
            .run()
            .await?;
            stored(db)
                .await?
                .ok_or_else(|| internal("the VAPID key is missing right after storing it"))?
        }
    };
    let key: StoredKey = keys.open_json(VAPID_AAD, &sealed)?;
    let mut public = vec![4u8];
    for coordinate in [&key.x, &key.y] {
        let bytes = webpush::decode(coordinate)
            .filter(|c| c.len() == 32)
            .ok_or_else(|| internal("the stored VAPID key is malformed"))?;
        public.extend_from_slice(&bytes);
    }
    let jwk = Object::new();
    set(&jwk, "kty", &"EC".into())?;
    set(&jwk, "crv", &"P-256".into())?;
    set(&jwk, "d", &key.d.as_str().into())?;
    set(&jwk, "x", &key.x.as_str().into())?;
    set(&jwk, "y", &key.y.as_str().into())?;
    let subtle = crypto::subtle()?;
    let usages = Array::of1(&"sign".into());
    let promise = subtle.import_key_with_object("jwk", &jwk, &p256("ECDSA")?, false, &usages)?;
    let signing: web_sys::CryptoKey = JsFuture::from(promise).await?.dyn_into()?;
    Ok(Vapid { signing, public })
}

/// The public key browsers subscribe with (base64url), or `None` when the
/// instance has no master key to keep a VAPID key with.
pub async fn public_key(db: &D1Database, cfg: &Config) -> AppResult<Option<String>> {
    let Ok(keys) = crypto::keys(cfg) else {
        return Ok(None);
    };
    Ok(Some(webpush::encode(&vapid(db, &keys).await?.public)))
}

impl Vapid {
    /// An ES256 signature of `data`: `r ‖ s`, as JWS wants it.
    async fn sign(&self, data: &[u8]) -> AppResult<Vec<u8>> {
        let algo = Object::new();
        set(&algo, "name", &"ECDSA".into())?;
        set(&algo, "hash", &"SHA-256".into())?;
        let promise =
            crypto::subtle()?.sign_with_object_and_u8_array(&algo, &self.signing, data)?;
        Ok(Uint8Array::new(&JsFuture::from(promise).await?).to_vec())
    }
}

/// A key pair for one message: its public key, and the secret it shares
/// with the browser whose public key is `theirs` (ECDH).
async fn agree(theirs: &[u8]) -> AppResult<(Vec<u8>, Vec<u8>)> {
    let subtle = crypto::subtle()?;
    let usages = Array::of1(&"deriveBits".into());
    // Extractable for its public half; the private one is used once and dropped.
    let promise = subtle.generate_key_with_object(&p256("ECDH")?, true, &usages)?;
    let pair: web_sys::CryptoKeyPair = JsFuture::from(promise).await?.unchecked_into();
    let ours = JsFuture::from(subtle.export_key("raw", &pair.get_public_key())?).await?;
    let promise = subtle.import_key_with_object(
        "raw",
        &Uint8Array::from(theirs),
        &p256("ECDH")?,
        true,
        &Array::new(),
    )?;
    let theirs: web_sys::CryptoKey = JsFuture::from(promise).await?.dyn_into()?;
    let derive = Object::new();
    set(&derive, "name", &"ECDH".into())?;
    set(&derive, "public", &theirs)?;
    let promise = subtle.derive_bits_with_object(&derive, &pair.get_private_key(), 256)?;
    let secret = JsFuture::from(promise).await?;
    Ok((
        Uint8Array::new(&ours).to_vec(),
        Uint8Array::new(&secret).to_vec(),
    ))
}

/// A browser to send to (`push_subscriptions`).
#[derive(Deserialize)]
struct DeviceRow {
    id: String,
    endpoint: String,
    p256dh: String,
    auth: String,
}

/// What signs this instance's messages: its VAPID key and contact.
struct Signer {
    vapid: Vapid,
    /// The `sub` claim: how a push service can reach whoever runs this instance.
    subject: String,
    now: i64,
}

impl Signer {
    async fn authorization(&self, endpoint: &str) -> AppResult<String> {
        let audience =
            webpush::audience(endpoint).ok_or_else(|| internal("a push endpoint has no origin"))?;
        let input = webpush::token_input(&audience, &self.subject, self.now);
        let signature = self.vapid.sign(input.as_bytes()).await?;
        Ok(webpush::authorization(
            &input,
            &signature,
            &self.vapid.public,
        ))
    }
}

/// Encrypts `payload` for one browser and hands it to its push service.
async fn send(signer: &Signer, device: &DeviceRow, payload: &[u8]) -> AppResult<Delivery> {
    let (Some(public_key), Some(auth)) = (
        webpush::decode(&device.p256dh),
        webpush::decode(&device.auth),
    ) else {
        return Err(internal("a stored push subscription has bad keys"));
    };
    let (ours, secret) = agree(&public_key).await?;
    let salt: [u8; 16] = random_bytes();
    let body = webpush::encrypt(
        payload,
        &Subscriber {
            public_key: &public_key,
            auth: &auth,
        },
        &Sender {
            public_key: &ours,
            ecdh_secret: &secret,
            salt: &salt,
        },
    )
    .map_err(internal)?;
    let headers = Headers::new();
    headers.set(
        "Authorization",
        &signer.authorization(&device.endpoint).await?,
    )?;
    headers.set("Content-Encoding", "aes128gcm")?;
    headers.set("Content-Type", "application/octet-stream")?;
    headers.set("TTL", &TTL_SECS.to_string())?;
    headers.set("Urgency", "normal")?;
    let mut init = RequestInit::new();
    // A push service answers itself: a redirect, which would take the
    // request to any address (another scheme, an IP address, another port),
    // counts as a failure.
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_redirect(RequestRedirect::Manual)
        .with_body(Some(Uint8Array::from(body.as_slice()).into()));
    let req = Request::new_with_init(&device.endpoint, &init)?;
    let status = Fetch::Request(req).send().await?.status_code();
    let delivery = webpush::delivery(status);
    if delivery != Delivery::Accepted {
        worker::console_error!(
            "push through {} answered HTTP {status}",
            webpush::service(&device.endpoint)
        );
    }
    Ok(delivery)
}

/// The VAPID contact: the instance's address, or fctl's when it has no https one.
fn subject(cfg: &Config) -> String {
    if cfg.public_url.starts_with("https://") {
        cfg.public_url.clone()
    } else {
        "https://fctl.sh".to_owned()
    }
}

/// Sends `message` to every browser where `user_id` turned notifications on.
/// Subscriptions their push service says are gone are deleted.
pub async fn deliver(
    cfg: &Config,
    db: &D1Database,
    user_id: &str,
    message: &PushMessage,
) -> AppResult<PushTestResult> {
    let devices: Vec<DeviceRow> = q(
        db,
        "SELECT id, endpoint, p256dh, auth FROM push_subscriptions WHERE user_id = ?",
        &[s(user_id)],
    )?
    .all()
    .await?;
    let mut result = PushTestResult { sent: 0, failed: 0 };
    if devices.is_empty() {
        return Ok(result);
    }
    let signer = Signer {
        vapid: vapid(db, &crypto::keys(cfg)?).await?,
        subject: subject(cfg),
        now: now_ms(),
    };
    let payload = serde_json::to_vec(message)?;
    // Subscriptions from before the endpoint rules of now that break them
    // (another port, say) go as gone ones do, unsent.
    let outcomes = futures::future::join_all(devices.iter().map(|d| async {
        if fctl_domain::validate::push_endpoint(&d.endpoint, cfg.dev).is_err() {
            return Ok(Delivery::Gone);
        }
        send(&signer, d, &payload).await
    }))
    .await;
    let mut sent: Vec<Val> = vec![n(signer.now)];
    let mut gone: Vec<Val> = Vec::new();
    for (device, outcome) in devices.iter().zip(outcomes) {
        match outcome {
            Ok(Delivery::Accepted) => {
                result.sent += 1;
                sent.push(s(&device.id));
            }
            Ok(Delivery::Gone) => {
                result.failed += 1;
                gone.push(s(&device.id));
            }
            Ok(Delivery::Failed) | Err(_) => result.failed += 1,
        }
    }
    let mut statements = Vec::new();
    if sent.len() > 1 {
        let ids = vec!["?"; sent.len() - 1].join(", ");
        statements.push(q(
            db,
            &format!("UPDATE push_subscriptions SET last_sent_at = ? WHERE id IN ({ids})"),
            &sent,
        )?);
    }
    if !gone.is_empty() {
        let ids = vec!["?"; gone.len()].join(", ");
        statements.push(q(
            db,
            &format!("DELETE FROM push_subscriptions WHERE id IN ({ids})"),
            &gone,
        )?);
    }
    if !statements.is_empty() {
        batch(db, statements).await?;
    }
    Ok(result)
}
