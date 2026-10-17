//! Cryptography for the control plane.
//!
//! - Secrets at rest (Claude credentials, project secrets) are sealed with
//!   AES-256-GCM under a key derived from `FCTL_MASTER_KEY`, with associated
//!   data naming the record, so ciphertext cannot be moved between records.
//!   Format: `v1.<base64url(nonce ‖ ciphertext ‖ tag)>`.
//! - Short-lived state (the OAuth cookie) is HMAC-SHA256 signed.
//! - RS256 (GitHub App JWTs) uses WebCrypto, which is constant time and fast.

use crate::error::{internal, AppError, AppResult};
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use fctl_domain::random_bytes;
use fctl_proto::ApiError;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

type HmacSha256 = Hmac<Sha256>;

/// Keys derived from the master key, one per purpose.
#[derive(Clone)]
pub struct Keys {
    enc: [u8; 32],
    mac: [u8; 32],
}

impl Keys {
    pub fn derive(master: &[u8; 32]) -> Self {
        Self {
            enc: hmac_raw(master, b"fctl/v1/encryption"),
            mac: hmac_raw(master, b"fctl/v1/mac"),
        }
    }

    /// Seals `plaintext`, binding it to `aad` (for example `claude:<id>`).
    pub fn seal(&self, aad: &str, plaintext: &[u8]) -> AppResult<String> {
        let cipher = Aes256Gcm::new_from_slice(&self.enc).map_err(internal)?;
        let nonce_bytes: [u8; 12] = random_bytes();
        let nonce = Nonce::from(nonce_bytes);
        let ct = cipher
            .encrypt(&nonce, Payload { msg: plaintext, aad: aad.as_bytes() })
            .map_err(|_| internal("encryption failed"))?;
        let mut out = Vec::with_capacity(12 + ct.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        Ok(format!("v1.{}", URL_SAFE_NO_PAD.encode(out)))
    }

    /// Opens a sealed value. Fails if it was sealed for different `aad`.
    pub fn open(&self, aad: &str, sealed: &str) -> AppResult<Vec<u8>> {
        let body = sealed
            .strip_prefix("v1.")
            .ok_or_else(|| internal("unknown ciphertext version"))?;
        let bytes = URL_SAFE_NO_PAD.decode(body).map_err(internal)?;
        if bytes.len() < 12 + 16 {
            return Err(internal("ciphertext too short"));
        }
        let (nonce_bytes, ct) = bytes.split_at(12);
        let nonce_arr: [u8; 12] = nonce_bytes.try_into().map_err(internal)?;
        let cipher = Aes256Gcm::new_from_slice(&self.enc).map_err(internal)?;
        cipher
            .decrypt(&Nonce::from(nonce_arr), Payload { msg: ct, aad: aad.as_bytes() })
            .map_err(|_| internal("decryption failed (wrong key or record)"))
    }

    pub fn seal_json<T: serde::Serialize>(&self, aad: &str, value: &T) -> AppResult<String> {
        self.seal(aad, &serde_json::to_vec(value)?)
    }

    pub fn open_json<T: serde::de::DeserializeOwned>(&self, aad: &str, sealed: &str) -> AppResult<T> {
        Ok(serde_json::from_slice(&self.open(aad, sealed)?)?)
    }

    /// `base64url(payload).base64url(hmac)`.
    pub fn sign(&self, payload: &[u8]) -> String {
        let p = URL_SAFE_NO_PAD.encode(payload);
        let tag = hmac_raw(&self.mac, p.as_bytes());
        format!("{p}.{}", URL_SAFE_NO_PAD.encode(tag))
    }

    pub fn verify(&self, signed: &str) -> Option<Vec<u8>> {
        let (p, tag) = signed.split_once('.')?;
        let tag = URL_SAFE_NO_PAD.decode(tag).ok()?;
        let mut mac = HmacSha256::new_from_slice(&self.mac).ok()?;
        mac.update(p.as_bytes());
        mac.verify_slice(&tag).ok()?;
        URL_SAFE_NO_PAD.decode(p).ok()
    }
}

fn hmac_raw(key: &[u8], data: &[u8]) -> [u8; 32] {
    // HMAC accepts keys of any length; new_from_slice cannot fail here.
    let mut mac = match HmacSha256::new_from_slice(key) {
        Ok(m) => m,
        Err(_) => return [0u8; 32],
    };
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// The instance's keys, or a clear error when `FCTL_MASTER_KEY` is missing.
pub fn keys(cfg: &crate::config::Config) -> AppResult<Keys> {
    cfg.master_key
        .as_ref()
        .map(Keys::derive)
        .ok_or_else(|| {
            AppError(ApiError::new(
                fctl_proto::ErrorCode::NotConfigured,
                "FCTL_MASTER_KEY is not set on this instance",
            ))
        })
}

pub fn b64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok()
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// A PKCE verifier and its S256 challenge.
pub fn pkce_pair() -> (String, String) {
    let verifier = b64url(&random_bytes::<32>());
    let challenge = b64url(&sha256(verifier.as_bytes()));
    (verifier, challenge)
}

fn subtle() -> AppResult<web_sys::SubtleCrypto> {
    let global = js_sys::global();
    let crypto = js_sys::Reflect::get(&global, &JsValue::from_str("crypto"))?
        .dyn_into::<web_sys::Crypto>()
        .map_err(|_| internal("crypto unavailable"))?;
    Ok(crypto.subtle())
}

/// Signs `data` with RSASSA-PKCS1-v1_5 / SHA-256 using a PKCS#8 PEM key.
pub async fn rs256_sign(pem: &str, data: &[u8]) -> AppResult<Vec<u8>> {
    let der = pem_to_der(pem)?;
    let subtle = subtle()?;
    let algo = js_sys::Object::new();
    js_sys::Reflect::set(&algo, &"name".into(), &"RSASSA-PKCS1-v1_5".into())?;
    js_sys::Reflect::set(&algo, &"hash".into(), &"SHA-256".into())?;
    let usages = js_sys::Array::of1(&"sign".into());
    let key_data = js_sys::Uint8Array::from(der.as_slice());
    let promise = subtle.import_key_with_object("pkcs8", &key_data, &algo, false, &usages)?;
    let key: web_sys::CryptoKey = JsFuture::from(promise).await?.dyn_into()?;
    let mut data_vec = data.to_vec();
    let sig = subtle.sign_with_str_and_u8_array("RSASSA-PKCS1-v1_5", &key, &mut data_vec)?;
    let buf = JsFuture::from(sig).await?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

fn pem_to_der(pem: &str) -> AppResult<Vec<u8>> {
    if pem.contains("BEGIN RSA PRIVATE KEY") {
        return Err(AppError(ApiError::new(
            fctl_proto::ErrorCode::NotConfigured,
            "GITHUB_APP_PRIVATE_KEY must be PKCS#8 (convert with: openssl pkcs8 -topk8 -nocrypt -in key.pem)",
        )));
    }
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .flat_map(|l| l.trim().chars())
        .collect();
    STANDARD.decode(body).map_err(internal)
}
