//! Id generation: `<prefix>_<ULID>`, so ids sort by creation time.

use crate::random_bytes;

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A 26-character ULID: 48 bits of milliseconds, then 80 random bits.
pub fn ulid(now_ms: i64) -> String {
    let rand: [u8; 10] = random_bytes();
    ulid_from_parts(now_ms, rand)
}

fn ulid_from_parts(now_ms: i64, rand: [u8; 10]) -> String {
    let ts = (now_ms.max(0) as u128) & ((1u128 << 48) - 1);
    let mut value: u128 = ts << 80;
    for (i, b) in rand.iter().enumerate() {
        value |= (*b as u128) << (8 * (9 - i));
    }
    let mut out = [0u8; 26];
    for i in (0..26).rev() {
        out[i] = CROCKFORD[(value & 0x1f) as usize];
        value >>= 5;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A new id with the given prefix, e.g. `new_id("usr", now)` → `usr_01J…`.
pub fn new_id(prefix: &str, now_ms: i64) -> String {
    format!("{prefix}_{}", ulid(now_ms))
}

macro_rules! id_ctor {
    ($fn:ident, $ty:ident) => {
        pub fn $fn(now_ms: i64) -> fctl_proto::$ty {
            fctl_proto::$ty(new_id(fctl_proto::$ty::PREFIX, now_ms))
        }
    };
}

id_ctor!(user_id, UserId);
id_ctor!(project_id, ProjectId);
id_ctor!(session_id, SessionId);
id_ctor!(runner_id, RunnerId);
id_ctor!(credential_id, CredentialId);
id_ctor!(allow_id, AllowId);
id_ctor!(token_id, TokenId);
id_ctor!(invite_id, InviteId);
id_ctor!(delivery_id, DeliveryId);
id_ctor!(login_job_id, LoginJobId);
id_ctor!(identity_id, IdentityId);

/// A random UUID v4 (for `claude --session-id`).
pub fn uuid_v4() -> String {
    let mut b: [u8; 16] = random_bytes();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulids_sort_by_time() {
        let a = ulid_from_parts(1_000, [0xff; 10]);
        let b = ulid_from_parts(1_001, [0x00; 10]);
        assert!(a < b);
        assert_eq!(a.len(), 26);
    }

    #[test]
    fn known_ulid() {
        assert_eq!(ulid_from_parts(0, [0; 10]), "00000000000000000000000000");
        assert_eq!(ulid_from_parts(1, [0; 10]), "00000000010000000000000000");
    }

    #[test]
    fn ids_validate() {
        let id = user_id(1_760_000_000_000);
        assert!(fctl_proto::UserId::is_valid(id.as_str()), "{id}");
        assert!(id.as_str().starts_with("usr_"));
    }

    #[test]
    fn uuid_shape() {
        let u = uuid_v4();
        assert_eq!(u.len(), 36);
        assert_eq!(u.as_bytes()[14], b'4');
    }
}
