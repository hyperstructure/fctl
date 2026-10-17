//! Opaque secrets: generation, hashing and recognition.
//!
//! Every secret has a type prefix so leaks are easy to spot and route
//! (`fctl_ws_…` web session, `fctl_cli_…`, `fctl_pat_…`, `fctl_rt_…` runner,
//! `fctl_inv_…` invitation, `fctl_rst_…` password reset, `fctl_dc_…` device
//! code). Only the SHA-256 of a secret is stored.

use crate::random_bytes;
use sha2::{Digest, Sha256};

const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretKind {
    WebSession,
    Cli,
    Pat,
    Runner,
    Invite,
    PasswordReset,
    DeviceCode,
    /// OAuth `state` and similar short-lived nonces.
    Nonce,
}

impl SecretKind {
    pub fn prefix(self) -> &'static str {
        match self {
            Self::WebSession => "fctl_ws_",
            Self::Cli => "fctl_cli_",
            Self::Pat => "fctl_pat_",
            Self::Runner => "fctl_rt_",
            Self::Invite => "fctl_inv_",
            Self::PasswordReset => "fctl_rst_",
            Self::DeviceCode => "fctl_dc_",
            Self::Nonce => "fctl_n_",
        }
    }

    pub fn of(secret: &str) -> Option<Self> {
        [
            Self::WebSession,
            Self::Cli,
            Self::Pat,
            Self::Runner,
            Self::Invite,
            Self::PasswordReset,
            Self::DeviceCode,
            Self::Nonce,
        ]
        .into_iter()
        .find(|k| secret.starts_with(k.prefix()))
    }
}

/// Encodes bytes in base62 (big-endian, fixed width for the input length).
fn base62(bytes: &[u8]) -> String {
    // Repeated division of a big-endian number; fine for 32 bytes.
    let mut digits: Vec<u8> = Vec::with_capacity(bytes.len() * 2);
    let mut num: Vec<u8> = bytes.to_vec();
    while num.iter().any(|&b| b != 0) {
        let mut rem: u32 = 0;
        for b in num.iter_mut() {
            let acc = (rem << 8) | (*b as u32);
            *b = (acc / 62) as u8;
            rem = acc % 62;
        }
        digits.push(BASE62[rem as usize]);
    }
    // 32 bytes need up to 43 base62 digits; pad for a fixed length.
    let width = (bytes.len() * 8 * 1000).div_ceil(5954);
    while digits.len() < width {
        digits.push(b'0');
    }
    digits.reverse();
    String::from_utf8_lossy(&digits).into_owned()
}

/// A new secret of the given kind: prefix + 256 random bits in base62.
pub fn new_secret(kind: SecretKind) -> String {
    let bytes: [u8; 32] = random_bytes();
    format!("{}{}", kind.prefix(), base62(&bytes))
}

/// Lowercase hex SHA-256 of a secret: what the database stores.
pub fn hash_secret(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

/// Whether a string has the shape of a secret of this kind.
pub fn looks_like(secret: &str, kind: SecretKind) -> bool {
    let Some(rest) = secret.strip_prefix(kind.prefix()) else {
        return false;
    };
    (40..=48).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Characters for codes people type: no vowels (no accidental words), no
/// look-alikes (0/O, 1/I/L).
const USER_CODE_ALPHABET: &[u8; 20] = b"BCDFGHJKMNPQRSTVWXZ2";

/// A device-flow user code like `KFTW-QXPL` (about 34 bits).
pub fn new_user_code() -> String {
    let bytes: [u8; 8] = random_bytes();
    let chars: Vec<u8> = bytes
        .iter()
        .map(|b| USER_CODE_ALPHABET[(*b as usize) % USER_CODE_ALPHABET.len()])
        .collect();
    format!(
        "{}-{}",
        String::from_utf8_lossy(&chars[..4]),
        String::from_utf8_lossy(&chars[4..])
    )
}

/// Normalizes what someone typed (`kftw qxpl`, `KFTWQXPL`) to `KFTW-QXPL`.
pub fn normalize_user_code(input: &str) -> Option<String> {
    let chars: Vec<u8> = input
        .bytes()
        .filter(|b| b.is_ascii_alphanumeric())
        .map(|b| b.to_ascii_uppercase())
        .collect();
    if chars.len() != 8 || !chars.iter().all(|c| USER_CODE_ALPHABET.contains(c)) {
        return None;
    }
    Some(format!(
        "{}-{}",
        String::from_utf8_lossy(&chars[..4]),
        String::from_utf8_lossy(&chars[4..])
    ))
}

/// Compares two byte strings in time that depends only on their lengths.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_have_prefix_and_fixed_length() {
        for _ in 0..50 {
            let s = new_secret(SecretKind::Runner);
            assert!(looks_like(&s, SecretKind::Runner), "{s}");
            assert_eq!(s.len(), "fctl_rt_".len() + 43, "{s}");
            assert_eq!(SecretKind::of(&s), Some(SecretKind::Runner));
        }
    }

    #[test]
    fn base62_known() {
        assert_eq!(base62(&[0, 0]).trim_start_matches('0'), "");
        assert_eq!(base62(&[0, 61]), "00Z".replace('Z', "z"));
        assert_eq!(base62(&[0, 62]), "010");
    }

    #[test]
    fn hashes_are_stable() {
        assert_eq!(
            hash_secret("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn user_codes_roundtrip() {
        let c = new_user_code();
        assert_eq!(c.len(), 9);
        assert_eq!(
            normalize_user_code(&c.to_lowercase().replace('-', " ")),
            Some(c)
        );
        assert_eq!(normalize_user_code("AEIO-UUUU"), None);
        assert_eq!(normalize_user_code("BCDF"), None);
    }

    #[test]
    fn ct_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
