//! Passkeys (WebAuthn Level 3): the checks on what a browser sends when a
//! passkey is made and when it signs in, without the signature itself, which
//! the control plane verifies with WebCrypto.
//!
//! fctl asks for no attestation: which authenticator made a passkey is the
//! person's business, so the attestation statement is ignored, and the
//! AAGUID only names the passkey ("1Password"). Every check here follows the
//! spec's §7.1 (registering) and §7.2 (signing in).

use crate::cbor::{self, Value};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// The algorithms fctl takes, by COSE number, most preferred first: ES256
/// (nearly every authenticator), EdDSA, RS256 (Windows Hello).
pub const ALGORITHMS: [i64; 3] = [ES256, EDDSA, RS256];
pub const ES256: i64 = -7;
pub const EDDSA: i64 = -8;
pub const RS256: i64 = -257;

/// Authenticator data flags (§6.1).
pub mod flags {
    /// User present: someone touched or looked at the device.
    pub const UP: u8 = 0x01;
    /// User verified: by fingerprint, face, PIN or screen lock.
    pub const UV: u8 = 0x04;
    /// Backup eligible: the passkey can sync between devices.
    pub const BE: u8 = 0x08;
    /// Backed up: it syncs now.
    pub const BS: u8 = 0x10;
    /// Attested credential data follows.
    pub const AT: u8 = 0x40;
    /// Extensions follow.
    pub const ED: u8 = 0x80;
}

/// Why a passkey's answer was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Something that isn't what browsers send.
    Malformed,
    /// Not the ceremony this was asked for (`webauthn.create` or `.get`).
    WrongType,
    /// Made on another site.
    WrongOrigin,
    /// For another relying party.
    WrongRp,
    /// Nobody was at the device.
    NotPresent,
    /// The device didn't verify who it was (no fingerprint, face or PIN).
    NotVerified,
    /// A key fctl can't check.
    UnsupportedKey,
}

pub fn b64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn from_b64url(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text.trim_end_matches('=')).ok()
}

/// The browser's account of the ceremony (`clientDataJSON`, §5.8.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientData {
    pub kind: String,
    pub challenge: Vec<u8>,
    pub origin: String,
    pub cross_origin: bool,
}

#[derive(serde::Deserialize)]
struct RawClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
    #[serde(default, rename = "crossOrigin")]
    cross_origin: bool,
}

/// Parses `clientDataJSON`.
pub fn client_data(json: &[u8]) -> Result<ClientData, Error> {
    let raw: RawClientData = serde_json::from_slice(json).map_err(|_| Error::Malformed)?;
    Ok(ClientData {
        kind: raw.kind,
        challenge: from_b64url(&raw.challenge).ok_or(Error::Malformed)?,
        origin: raw.origin,
        cross_origin: raw.cross_origin,
    })
}

/// A passkey's public key (COSE, RFC 9053).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublicKey {
    /// ES256: a P-256 point.
    Ec2 { x: Vec<u8>, y: Vec<u8> },
    /// EdDSA: an Ed25519 key.
    Okp { x: Vec<u8> },
    /// RS256: an RSA modulus and exponent.
    Rsa { n: Vec<u8>, e: Vec<u8> },
}

impl PublicKey {
    /// Its COSE algorithm.
    pub fn algorithm(&self) -> i64 {
        match self {
            Self::Ec2 { .. } => ES256,
            Self::Okp { .. } => EDDSA,
            Self::Rsa { .. } => RS256,
        }
    }
}

/// Parses a COSE key, as stored or as the authenticator sent it.
pub fn public_key(cose: &Value) -> Result<PublicKey, Error> {
    let int = |k: i128| cose.get_int(k).and_then(Value::as_int);
    let bytes = |k: i128| {
        cose.get_int(k)
            .and_then(Value::as_bytes)
            .map(<[u8]>::to_vec)
    };
    let (kty, alg) = (int(1), int(3));
    match (kty, alg) {
        // EC2 on P-256 (crv 1), ES256.
        (Some(2), Some(a)) if a == i128::from(ES256) => {
            let (x, y) = (bytes(-2), bytes(-3));
            match (int(-1), x, y) {
                (Some(1), Some(x), Some(y)) if x.len() == 32 && y.len() == 32 => {
                    Ok(PublicKey::Ec2 { x, y })
                }
                _ => Err(Error::UnsupportedKey),
            }
        }
        // OKP on Ed25519 (crv 6), EdDSA.
        (Some(1), Some(a)) if a == i128::from(EDDSA) => match (int(-1), bytes(-2)) {
            (Some(6), Some(x)) if x.len() == 32 => Ok(PublicKey::Okp { x }),
            _ => Err(Error::UnsupportedKey),
        },
        // RSA, RS256, of at least 2048 bits.
        (Some(3), Some(a)) if a == i128::from(RS256) => match (bytes(-1), bytes(-2)) {
            (Some(n), Some(e)) if n.len() >= 256 && !e.is_empty() => Ok(PublicKey::Rsa { n, e }),
            _ => Err(Error::UnsupportedKey),
        },
        _ => Err(Error::UnsupportedKey),
    }
}

/// The authenticator's data (§6.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthData {
    pub rp_id_hash: [u8; 32],
    pub flags: u8,
    pub sign_count: u32,
    /// When a passkey was made: its model, id and public key.
    pub attested: Option<Attested>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attested {
    /// The authenticator's model, which names the passkey's provider.
    pub aaguid: [u8; 16],
    pub credential_id: Vec<u8>,
    pub public_key: PublicKey,
    /// The COSE key as the authenticator encoded it, to store.
    pub public_key_cose: Vec<u8>,
}

impl AuthData {
    pub fn has(&self, flag: u8) -> bool {
        self.flags & flag == flag
    }
}

/// Parses authenticator data.
pub fn auth_data(bytes: &[u8]) -> Result<AuthData, Error> {
    let head = bytes.get(..37).ok_or(Error::Malformed)?;
    let mut rp_id_hash = [0u8; 32];
    rp_id_hash.copy_from_slice(&head[..32]);
    let flags = head[32];
    let sign_count = u32::from_be_bytes([head[33], head[34], head[35], head[36]]);
    let mut rest = &bytes[37..];
    let attested = if flags & flags::AT != 0 {
        let aaguid: [u8; 16] = rest
            .get(..16)
            .and_then(|a| a.try_into().ok())
            .ok_or(Error::Malformed)?;
        let len = rest
            .get(16..18)
            .map(|l| usize::from(u16::from_be_bytes([l[0], l[1]])))
            .ok_or(Error::Malformed)?;
        let credential_id = rest.get(18..18 + len).ok_or(Error::Malformed)?.to_vec();
        if credential_id.is_empty() || credential_id.len() > 1023 {
            return Err(Error::Malformed);
        }
        rest = &rest[18 + len..];
        let (cose, used) = cbor::decode(rest).map_err(|_| Error::Malformed)?;
        let public_key_cose = rest[..used].to_vec();
        rest = &rest[used..];
        Some(Attested {
            aaguid,
            credential_id,
            public_key: public_key(&cose)?,
            public_key_cose,
        })
    } else {
        None
    };
    if flags & flags::ED != 0 {
        let (_, used) = cbor::decode(rest).map_err(|_| Error::Malformed)?;
        rest = &rest[used..];
    }
    if !rest.is_empty() {
        return Err(Error::Malformed);
    }
    Ok(AuthData {
        rp_id_hash,
        flags,
        sign_count,
        attested,
    })
}

/// What a relying party expects of a ceremony.
pub struct Expect<'a> {
    /// The relying party's id: the app's host name.
    pub rp_id: &'a str,
    /// Whether an origin is the app's own.
    pub origin: &'a dyn Fn(&str) -> bool,
    /// The device must verify who it is (fingerprint, face, PIN).
    pub user_verification: bool,
}

fn check_common(cd: &ClientData, kind: &str, ad: &AuthData, expect: &Expect) -> Result<(), Error> {
    if cd.kind != kind {
        return Err(Error::WrongType);
    }
    if cd.cross_origin || !(expect.origin)(&cd.origin) {
        return Err(Error::WrongOrigin);
    }
    let rp: [u8; 32] = Sha256::digest(expect.rp_id.as_bytes()).into();
    if ad.rp_id_hash != rp {
        return Err(Error::WrongRp);
    }
    if !ad.has(flags::UP) {
        return Err(Error::NotPresent);
    }
    if expect.user_verification && !ad.has(flags::UV) {
        return Err(Error::NotVerified);
    }
    // Backed up implies backup eligible (§6.1).
    if ad.has(flags::BS) && !ad.has(flags::BE) {
        return Err(Error::Malformed);
    }
    Ok(())
}

/// A passkey just made, checked (§7.1); the caller has matched its
/// challenge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub credential_id: Vec<u8>,
    pub public_key: PublicKey,
    pub public_key_cose: Vec<u8>,
    pub aaguid: [u8; 16],
    pub sign_count: u32,
    /// It syncs between the person's devices.
    pub backed_up: bool,
}

/// Checks `attestationObject` for a passkey being made.
pub fn check_registration(
    cd: &ClientData,
    attestation_object: &[u8],
    expect: &Expect,
) -> Result<Registration, Error> {
    let object = cbor::decode_all(attestation_object).map_err(|_| Error::Malformed)?;
    let ad = object
        .get_text("authData")
        .and_then(Value::as_bytes)
        .ok_or(Error::Malformed)
        .and_then(auth_data)?;
    check_common(cd, "webauthn.create", &ad, expect)?;
    let attested = ad.attested.ok_or(Error::Malformed)?;
    Ok(Registration {
        credential_id: attested.credential_id,
        public_key: attested.public_key,
        public_key_cose: attested.public_key_cose,
        aaguid: attested.aaguid,
        sign_count: ad.sign_count,
        backed_up: ad.flags & flags::BS != 0,
    })
}

/// A passkey's signature to check, after the checks on the rest (§7.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assertion {
    pub sign_count: u32,
    pub backed_up: bool,
    /// What the signature covers: the authenticator data, then the SHA-256
    /// of `clientDataJSON`.
    pub signed: Vec<u8>,
}

/// Checks a sign-in's authenticator data; the signature over
/// [`Assertion::signed`] is left to verify.
pub fn check_assertion(
    cd: &ClientData,
    client_data_json: &[u8],
    authenticator_data: &[u8],
    expect: &Expect,
) -> Result<Assertion, Error> {
    let ad = auth_data(authenticator_data)?;
    check_common(cd, "webauthn.get", &ad, expect)?;
    let mut signed = authenticator_data.to_vec();
    signed.extend_from_slice(&Sha256::digest(client_data_json));
    Ok(Assertion {
        sign_count: ad.sign_count,
        backed_up: ad.flags & flags::BS != 0,
        signed,
    })
}

/// Whether a signature counter moved on as it should: most synced passkeys
/// always send 0; others count up, and one that didn't may have been cloned.
pub fn counter_ok(stored: u32, received: u32) -> bool {
    (stored == 0 && received == 0) || received > stored
}

/// An ECDSA signature in DER, as WebAuthn sends it, as the `r ‖ s` that
/// WebCrypto takes (32 bytes each, for P-256).
pub fn der_to_raw(der: &[u8]) -> Option<[u8; 64]> {
    // SEQUENCE { INTEGER r, INTEGER s }, all lengths short.
    let (&0x30, rest) = der.split_first()? else {
        return None;
    };
    let (&len, rest) = rest.split_first()?;
    if usize::from(len) != rest.len() {
        return None;
    }
    let mut out = [0u8; 64];
    let mut rest = rest;
    for half in out.chunks_mut(32) {
        let (&0x02, r) = rest.split_first()? else {
            return None;
        };
        let (&n, r) = r.split_first()?;
        let n = usize::from(n);
        let int = r.get(..n)?;
        rest = r.get(n..)?;
        // Big-endian, with a leading 0 when the top bit is set.
        let int = match int {
            [0, tail @ ..] if tail.len() == 32 => tail,
            _ if int.len() <= 32 => int,
            _ => return None,
        };
        half[32 - int.len()..].copy_from_slice(int);
    }
    rest.is_empty().then_some(out)
}

/// An AAGUID as a UUID: `fbfc3007-154e-4ecc-8c0b-6e020557d7bd`.
pub fn uuid(aaguid: &[u8; 16]) -> String {
    let h: String = aaguid.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// The passkey provider an AAGUID names, for the passkey's default name.
/// Providers that don't say (all zeros) or aren't listed get none.
pub fn provider(aaguid: &str) -> Option<&'static str> {
    Some(match aaguid {
        "fbfc3007-154e-4ecc-8c0b-6e020557d7bd" => "Apple Passwords",
        "dd4ec289-e01d-41c9-bb89-70fa845d4bf2" => "iCloud Keychain",
        "ea9b8d66-4d01-1d21-3ce4-b6b48cb575d4" => "Google Password Manager",
        "adce0002-35bc-c60a-648b-0b25f1f05503" => "Chrome on Mac",
        "771b48fd-d3d4-4f74-9232-fc157ab0507a" => "Edge on Mac",
        "b5397666-4885-aa6b-cebf-e52262a439a2" => "Chromium",
        "08987058-cadc-4b81-b6e1-30de50dcbe96"
        | "9ddd1817-af5a-4672-a2b9-3e3dd95000a9"
        | "6028b017-b1d4-4c02-b4b3-afcdafc96bb2" => "Windows Hello",
        "d3452668-01fd-4c12-926c-83a4204853aa" => "Microsoft Password Manager",
        "bada5566-a7aa-401f-bd96-45619a55120d" => "1Password",
        "d548826e-79b4-db40-a3d8-11116f7e8349" => "Bitwarden",
        "531126d6-e717-415c-9320-3d9aa6981239" => "Dashlane",
        "b84e4048-15dc-4dd0-8640-f4f60813c8af" => "NordPass",
        "0ea242b4-43c4-4a1b-8b17-dd6d0b6baec6" => "Keeper",
        "f3809540-7f14-49c1-a8b3-8f813b225541" => "Enpass",
        "b78a0a55-6ef8-d246-a042-ba0f6d55050c" => "LastPass",
        "50726f74-6f6e-5061-7373-50726f746f6e" => "Proton Pass",
        "fdb141b2-5d84-443e-8a35-4698c205a502" => "KeePassXC",
        "53414d53-554e-4700-0000-000000000000" => "Samsung Pass",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CBOR for a COSE ES256 key with the given coordinates.
    fn cose_es256(x: &[u8; 32], y: &[u8; 32]) -> Vec<u8> {
        let mut out = vec![0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20];
        out.extend_from_slice(x);
        out.extend_from_slice(&[0x22, 0x58, 0x20]);
        out.extend_from_slice(y);
        out
    }

    fn auth_data_bytes(rp_id: &str, flags: u8, count: u32, attested: Option<&[u8]>) -> Vec<u8> {
        let mut out: Vec<u8> = Sha256::digest(rp_id.as_bytes()).to_vec();
        out.push(flags);
        out.extend_from_slice(&count.to_be_bytes());
        if let Some(cose) = attested {
            out.extend_from_slice(&[0xad; 16]);
            out.extend_from_slice(&[0, 4, 1, 2, 3, 4]);
            out.extend_from_slice(cose);
        }
        out
    }

    fn client(kind: &str, origin: &str) -> Vec<u8> {
        format!(
            r#"{{"type":"{kind}","challenge":"{}","origin":"{origin}","crossOrigin":false}}"#,
            b64url(b"challenge")
        )
        .into_bytes()
    }

    fn attestation(auth_data: &[u8]) -> Vec<u8> {
        // {"fmt": "none", "attStmt": {}, "authData": h'…'}
        let mut out = vec![0xa3, 0x63, b'f', b'm', b't', 0x64, b'n', b'o', b'n', b'e'];
        out.extend_from_slice(&[0x67]);
        out.extend_from_slice(b"attStmt");
        out.push(0xa0);
        out.push(0x68);
        out.extend_from_slice(b"authData");
        out.extend_from_slice(&[0x59, (auth_data.len() >> 8) as u8, auth_data.len() as u8]);
        out.extend_from_slice(auth_data);
        out
    }

    fn expect<'a>(origin: &'a dyn Fn(&str) -> bool) -> Expect<'a> {
        Expect {
            rp_id: "app.fctl.sh",
            origin,
            user_verification: true,
        }
    }

    const ORIGIN: &str = "https://app.fctl.sh";

    #[test]
    fn registrations_check_out() {
        let cose = cose_es256(&[1; 32], &[2; 32]);
        let ad = auth_data_bytes(
            "app.fctl.sh",
            flags::UP | flags::UV | flags::BE | flags::BS | flags::AT,
            0,
            Some(&cose),
        );
        let json = client("webauthn.create", ORIGIN);
        let cd = client_data(&json).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(cd.challenge, b"challenge");
        let same = |o: &str| o == ORIGIN;
        let r = check_registration(&cd, &attestation(&ad), &expect(&same))
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(r.credential_id, vec![1, 2, 3, 4]);
        assert_eq!(
            r.public_key,
            PublicKey::Ec2 {
                x: vec![1; 32],
                y: vec![2; 32]
            }
        );
        assert_eq!(r.public_key.algorithm(), ES256);
        assert_eq!(r.public_key_cose, cose);
        assert!(r.backed_up);
        assert_eq!(uuid(&r.aaguid), "adadadad-adad-adad-adad-adadadadadad");
    }

    #[test]
    fn registrations_from_elsewhere_or_unverified_are_refused() {
        let cose = cose_es256(&[1; 32], &[2; 32]);
        let good = auth_data_bytes(
            "app.fctl.sh",
            flags::UP | flags::UV | flags::AT,
            0,
            Some(&cose),
        );
        let same = |o: &str| o == ORIGIN;
        let e = expect(&same);
        let check = |kind: &str, origin: &str, ad: &[u8]| {
            let json = client(kind, origin);
            let cd = client_data(&json).unwrap_or_else(|e| panic!("{e:?}"));
            check_registration(&cd, &attestation(ad), &e).err()
        };
        assert_eq!(check("webauthn.create", ORIGIN, &good), None);
        assert_eq!(check("webauthn.get", ORIGIN, &good), Some(Error::WrongType));
        assert_eq!(
            check("webauthn.create", "https://evil.example", &good),
            Some(Error::WrongOrigin)
        );
        let other_rp = auth_data_bytes(
            "evil.example",
            flags::UP | flags::UV | flags::AT,
            0,
            Some(&cose),
        );
        assert_eq!(
            check("webauthn.create", ORIGIN, &other_rp),
            Some(Error::WrongRp)
        );
        let absent = auth_data_bytes("app.fctl.sh", flags::UV | flags::AT, 0, Some(&cose));
        assert_eq!(
            check("webauthn.create", ORIGIN, &absent),
            Some(Error::NotPresent)
        );
        let unverified = auth_data_bytes("app.fctl.sh", flags::UP | flags::AT, 0, Some(&cose));
        assert_eq!(
            check("webauthn.create", ORIGIN, &unverified),
            Some(Error::NotVerified)
        );
        let no_key = auth_data_bytes("app.fctl.sh", flags::UP | flags::UV, 0, None);
        assert_eq!(
            check("webauthn.create", ORIGIN, &no_key),
            Some(Error::Malformed)
        );
        let synced_not_eligible = auth_data_bytes(
            "app.fctl.sh",
            flags::UP | flags::UV | flags::BS | flags::AT,
            0,
            Some(&cose),
        );
        assert_eq!(
            check("webauthn.create", ORIGIN, &synced_not_eligible),
            Some(Error::Malformed)
        );
        // Trailing bytes after the key, without the extensions flag.
        let mut trailing = good.clone();
        trailing.push(0);
        assert_eq!(
            check("webauthn.create", ORIGIN, &trailing),
            Some(Error::Malformed)
        );
    }

    #[test]
    fn sign_ins_sign_the_data_and_the_client_data_hash() {
        let ad = auth_data_bytes("app.fctl.sh", flags::UP | flags::UV, 7, None);
        let json = client("webauthn.get", ORIGIN);
        let cd = client_data(&json).unwrap_or_else(|e| panic!("{e:?}"));
        let same = |o: &str| o == ORIGIN;
        let a =
            check_assertion(&cd, &json, &ad, &expect(&same)).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(a.sign_count, 7);
        assert_eq!(&a.signed[..37], &ad[..]);
        assert_eq!(&a.signed[37..], Sha256::digest(&json).as_slice());
        let created = client("webauthn.create", ORIGIN);
        let cd = client_data(&created).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            check_assertion(&cd, &created, &ad, &expect(&same)).err(),
            Some(Error::WrongType)
        );
    }

    #[test]
    fn keys_fctl_cannot_check_are_refused() {
        // EC2 on P-384 (crv 2).
        let mut p384 = cose_es256(&[1; 32], &[2; 32]);
        p384[6] = 0x02;
        let key = cbor::decode_all(&p384).unwrap_or(Value::Null);
        assert_eq!(public_key(&key), Err(Error::UnsupportedKey));
        // Ed25519.
        let mut okp = vec![0xa4, 0x01, 0x01, 0x03, 0x27, 0x20, 0x06, 0x21, 0x58, 0x20];
        okp.extend_from_slice(&[9; 32]);
        let key = cbor::decode_all(&okp).unwrap_or(Value::Null);
        assert_eq!(public_key(&key), Ok(PublicKey::Okp { x: vec![9; 32] }));
        // RSA with a 1024-bit modulus is too weak.
        let mut rsa = vec![0xa4, 0x01, 0x03, 0x03, 0x39, 0x01, 0x00, 0x20, 0x58, 0x80];
        rsa.extend_from_slice(&[0xff; 128]);
        rsa.extend_from_slice(&[0x21, 0x43, 0x01, 0x00, 0x01]);
        let key = cbor::decode_all(&rsa).unwrap_or(Value::Null);
        assert_eq!(public_key(&key), Err(Error::UnsupportedKey));
    }

    #[test]
    fn counters_must_move_on_unless_both_are_zero() {
        assert!(counter_ok(0, 0));
        assert!(counter_ok(0, 1));
        assert!(counter_ok(5, 6));
        assert!(!counter_ok(5, 5));
        assert!(!counter_ok(5, 0));
    }

    #[test]
    fn der_signatures_become_raw() {
        // r with a leading zero (top bit set), s short.
        let mut der = vec![0x30, 0x27, 0x02, 0x21, 0x00];
        der.extend_from_slice(&[0x80; 32]);
        der.extend_from_slice(&[0x02, 0x02, 0x01, 0x02]);
        let raw = der_to_raw(&der).unwrap_or([0; 64]);
        assert_eq!(&raw[..32], &[0x80; 32]);
        assert_eq!(&raw[32..62], &[0; 30]);
        assert_eq!(&raw[62..], &[1, 2]);
        assert_eq!(der_to_raw(&der[..der.len() - 1]), None);
        assert_eq!(der_to_raw(&[0x31, 0x00]), None);
    }

    #[test]
    fn providers_are_named_by_aaguid() {
        assert_eq!(
            provider("bada5566-a7aa-401f-bd96-45619a55120d"),
            Some("1Password")
        );
        assert_eq!(provider("00000000-0000-0000-0000-000000000000"), None);
    }
}
