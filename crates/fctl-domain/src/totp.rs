//! Time-based one-time passwords (TOTP, RFC 6238): the codes authenticator
//! apps show, for two-factor sign-in. HMAC-SHA1, six digits and 30-second
//! steps, the parameters every app supports, so links leave them out.

use crate::random_bytes;
use crate::tokens::constant_time_eq;
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

/// Seconds a code lasts.
pub const PERIOD_SECS: i64 = 30;
/// Digits in a code.
pub const DIGITS: usize = 6;
/// Bytes of a secret: 160 bits, as RFC 4226 recommends.
pub const SECRET_BYTES: usize = 20;
/// Steps on either side of now whose codes still count, for phones whose
/// clocks are a little off.
const SKEW: i64 = 1;

const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// A new secret.
pub fn new_secret() -> [u8; SECRET_BYTES] {
    random_bytes()
}

/// The time step `now_ms` falls in.
pub fn step(now_ms: i64) -> i64 {
    now_ms.div_euclid(PERIOD_SECS * 1000)
}

/// When the step after the one `now_ms` falls in starts: when the code an
/// app shows changes.
pub fn next_step_at(now_ms: i64) -> i64 {
    (step(now_ms) + 1) * PERIOD_SECS * 1000
}

/// The code of a time step: HOTP (RFC 4226) with the step as its counter.
pub fn code(secret: &[u8], step: i64) -> u32 {
    let Ok(mut mac) = <Hmac<Sha1> as KeyInit>::new_from_slice(secret) else {
        // HMAC takes keys of any length.
        return 0;
    };
    mac.update(&(step as u64).to_be_bytes());
    let digest: [u8; 20] = mac.finalize().into_bytes().into();
    // Dynamic truncation: the low nibble of the last byte picks four bytes.
    let offset = usize::from(digest[19] & 0x0f);
    let mut word = [0u8; 4];
    word.copy_from_slice(&digest[offset..offset + 4]);
    (u32::from_be_bytes(word) & 0x7fff_ffff) % 10u32.pow(DIGITS as u32)
}

/// [`code`] as the app shows it, with leading zeros.
pub fn code_text(secret: &[u8], step: i64) -> String {
    format!("{:0width$}", code(secret, step), width = DIGITS)
}

/// What someone typed, without the spaces and dashes apps and people put
/// in it, if it is a code at all: six digits.
pub fn normalize(typed: &str) -> Option<String> {
    let digits: String = typed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    (digits.len() == DIGITS && digits.bytes().all(|b| b.is_ascii_digit())).then_some(digits)
}

/// How a typed code compares with the secret's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// It is the code of this step, which no code was used for yet.
    Valid(i64),
    /// It is right, but a code of this step or a later one was already
    /// used: each code works once.
    Used,
    Wrong,
}

/// Checks a typed code against the steps around `now_ms`. `last_step` is
/// the step of the last code used (0 for none), so a code can't be used
/// twice, nor one older than the last. Compares in constant time.
pub fn check(secret: &[u8], typed: &str, now_ms: i64, last_step: i64) -> Check {
    let Some(typed) = normalize(typed) else {
        return Check::Wrong;
    };
    let now = step(now_ms);
    let mut result = Check::Wrong;
    for s in (now - SKEW)..=(now + SKEW) {
        let matches = constant_time_eq(code_text(secret, s).as_bytes(), typed.as_bytes());
        result = match (matches, result) {
            (true, Check::Wrong | Check::Used) if s > last_step => Check::Valid(s),
            (true, Check::Wrong) => Check::Used,
            (_, r) => r,
        };
    }
    result
}

/// RFC 4648 base32 without padding: how secrets are shown and put in links.
pub fn base32(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for &b in bytes {
        buffer = (buffer << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(BASE32[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(BASE32[((buffer << (5 - bits)) & 31) as usize]));
    }
    out
}

/// Decodes [`base32`], in either case, ignoring spaces and padding.
pub fn from_base32(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 5 / 8);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for c in text
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
    {
        let value = BASE32.iter().position(|&a| a == c.to_ascii_uppercase())?;
        buffer = (buffer << 5) | value as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

/// A secret in groups of four characters, for typing it in by hand.
pub fn grouped(secret_b32: &str) -> String {
    secret_b32
        .as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The link authenticator apps take, in Google's key URI format:
/// `otpauth://totp/Issuer:account?secret=…&issuer=Issuer`. Apps show the
/// issuer as the account's title.
pub fn uri(issuer: &str, account: &str, secret_b32: &str) -> String {
    let issuer = percent(issuer);
    format!(
        "otpauth://totp/{issuer}:{}?secret={secret_b32}&issuer={issuer}",
        percent(account)
    )
}

/// Percent-encodes all but RFC 3986's unreserved characters (spaces become
/// `%20`, which every app reads, where some show a `+` as it is).
pub(crate) fn percent(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SHA-1 secret of RFC 6238's test vectors.
    const RFC_SECRET: &[u8] = b"12345678901234567890";

    #[test]
    fn rfc6238_vectors() {
        // RFC 6238 appendix B lists eight digits; six are their last six.
        for (secs, expected) in [
            (59_i64, "287082"),
            (1_111_111_109, "081804"),
            (1_111_111_111, "050471"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
            (20_000_000_000, "353130"),
        ] {
            assert_eq!(code_text(RFC_SECRET, step(secs * 1000)), expected, "{secs}");
        }
    }

    #[test]
    fn rfc4226_vectors() {
        let expected = [
            755224, 287082, 359152, 969429, 338314, 254676, 287922, 162583, 399871, 520489,
        ];
        for (counter, code_value) in expected.iter().enumerate() {
            assert_eq!(code(RFC_SECRET, counter as i64), *code_value);
        }
    }

    #[test]
    fn base32_round_trips_rfc4648_vectors() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "MY"),
            ("fo", "MZXQ"),
            ("foo", "MZXW6"),
            ("foob", "MZXW6YQ"),
            ("fooba", "MZXW6YTB"),
            ("foobar", "MZXW6YTBOI"),
        ] {
            assert_eq!(base32(plain.as_bytes()), encoded);
            assert_eq!(from_base32(encoded).as_deref(), Some(plain.as_bytes()));
        }
        assert_eq!(
            from_base32("mzxw 6ytb oi======").as_deref(),
            Some(&b"foobar"[..])
        );
        assert_eq!(from_base32("MZXW1"), None);
        let secret = new_secret();
        let text = base32(&secret);
        assert_eq!(text.len(), 32);
        assert_eq!(from_base32(&text).as_deref(), Some(&secret[..]));
    }

    #[test]
    fn codes_of_the_steps_around_now_count_once() {
        let now = 1_700_000_000_000;
        let s = step(now);
        let current = code_text(RFC_SECRET, s);
        assert_eq!(check(RFC_SECRET, &current, now, 0), Check::Valid(s));
        assert_eq!(
            check(RFC_SECRET, &code_text(RFC_SECRET, s - 1), now, 0),
            Check::Valid(s - 1)
        );
        assert_eq!(
            check(RFC_SECRET, &code_text(RFC_SECRET, s + 1), now, 0),
            Check::Valid(s + 1)
        );
        assert_eq!(
            check(RFC_SECRET, &code_text(RFC_SECRET, s - 2), now, 0),
            Check::Wrong
        );
        assert_eq!(
            check(RFC_SECRET, &code_text(RFC_SECRET, s + 2), now, 0),
            Check::Wrong
        );
        // Used once: the same code, and older ones, are refused.
        assert_eq!(check(RFC_SECRET, &current, now, s), Check::Used);
        assert_eq!(
            check(RFC_SECRET, &code_text(RFC_SECRET, s - 1), now, s),
            Check::Used
        );
        // The next one still works.
        assert_eq!(
            check(RFC_SECRET, &code_text(RFC_SECRET, s + 1), now, s),
            Check::Valid(s + 1)
        );
    }

    #[test]
    fn typed_codes_are_forgiving_about_spaces_only() {
        let now = 1_700_000_000_000;
        let s = step(now);
        let c = code_text(RFC_SECRET, s);
        let spaced = format!(" {} {} ", &c[..3], &c[3..]);
        assert_eq!(check(RFC_SECRET, &spaced, now, 0), Check::Valid(s));
        assert_eq!(normalize("123-456").as_deref(), Some("123456"));
        assert_eq!(normalize("12345"), None);
        assert_eq!(normalize("1234567"), None);
        assert_eq!(normalize("12345a"), None);
        assert_eq!(normalize("１２３４５６"), None);
        assert_eq!(check(RFC_SECRET, "", now, 0), Check::Wrong);
    }

    #[test]
    fn steps_change_every_thirty_seconds() {
        assert_eq!(step(0), 0);
        assert_eq!(step(29_999), 0);
        assert_eq!(step(30_000), 1);
        assert_eq!(next_step_at(29_999), 30_000);
        assert_eq!(next_step_at(30_000), 60_000);
    }

    #[test]
    fn links_encode_the_issuer_and_account() {
        assert_eq!(
            uri("fctl dev", "ada@example.com", "JBSWY3DPEHPK3PXP"),
            "otpauth://totp/fctl%20dev:ada%40example.com?secret=JBSWY3DPEHPK3PXP&issuer=fctl%20dev"
        );
        assert_eq!(
            uri("Acme: fctl", "a+b@x.io", "ABC"),
            "otpauth://totp/Acme%3A%20fctl:a%2Bb%40x.io?secret=ABC&issuer=Acme%3A%20fctl"
        );
    }

    #[test]
    fn secrets_group_in_fours() {
        assert_eq!(grouped("JBSWY3DPEHPK3PXP"), "JBSW Y3DP EHPK 3PXP");
        assert_eq!(grouped("ABCDEF"), "ABCD EF");
        assert_eq!(grouped(""), "");
    }
}
