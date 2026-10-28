//! Workspaces' and projects' slugs, their addresses (`/w/<slug>`,
//! `/p/<slug>`). A new one takes the slug its name makes while that is
//! free; when another workspace or project has it, or fctl
//! ([`validate::RESERVED_SLUGS`]) or an instance admin reserves it, it gets
//! a random ending instead: `example-829d34`. Nobody chooses a reserved one
//! as an address, and those that have one already keep it.

use crate::validate::{self, Result};
use fctl_proto::ApiError;

/// The longest slug ([`validate::slug`]).
pub const MAX_LEN: usize = 48;

/// `base` with a random ending of six hex digits (`example-829d34`),
/// shortened to leave room for it.
pub fn with_ending(base: &str, random: [u8; 3]) -> String {
    let ending = hex::encode(random);
    let room = MAX_LEN - ending.len() - 1;
    let base: String = base.chars().take(room).collect();
    match base.trim_end_matches('-') {
        "" => ending,
        base => format!("{base}-{ending}"),
    }
}

/// A slug for an instance admin to reserve: one a workspace or project
/// could take, and not one of fctl's own, which it reserves itself.
pub fn reservable(input: &str) -> Result<String> {
    let slug = validate::slug_shape(input)?;
    if validate::RESERVED_SLUGS.contains(&slug.as_str()) {
        return Err(ApiError::conflict(format!("fctl reserves {slug} itself")));
    }
    Ok(slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endings_are_six_hex_digits() {
        assert_eq!(with_ending("example", [0x82, 0x9d, 0x34]), "example-829d34");
        assert_eq!(with_ending("new", [0, 0, 0x0f]), "new-00000f");
        assert_eq!(with_ending("", [0xab, 0xcd, 0xef]), "abcdef");
    }

    #[test]
    fn long_bases_make_room_for_the_ending() {
        let long = "a".repeat(MAX_LEN);
        let slug = with_ending(&long, [1, 2, 3]);
        assert_eq!(slug.len(), MAX_LEN);
        assert!(slug.ends_with("-010203"), "{slug}");
        // Cut just after a dash, the base loses it: no double dash.
        let dashed = format!("{}-{}", "a".repeat(40), "b".repeat(7));
        let slug = with_ending(&dashed, [1, 2, 3]);
        assert_eq!(slug, format!("{}-010203", "a".repeat(40)));
        for base in [long.as_str(), dashed.as_str(), "example", "x"] {
            let slug = with_ending(base, [0xff, 0, 0x10]);
            assert_eq!(validate::slug(&slug).ok(), Some(slug.clone()), "{base}");
        }
    }

    #[test]
    fn admins_reserve_slugs_fctl_doesnt() {
        assert_eq!(reservable(" Home ").ok().as_deref(), Some("home"));
        assert_eq!(reservable("me").ok().as_deref(), Some("me"));
        let own = reservable("settings").err();
        assert_eq!(
            own.as_ref().map(|e| e.code),
            Some(fctl_proto::ErrorCode::Conflict)
        );
        assert_eq!(
            own.map(|e| e.message).as_deref(),
            Some("fctl reserves settings itself")
        );
        for bad in ["a", "/w/home", "a--b", "über"] {
            assert_eq!(
                reservable(bad).err().map(|e| e.code),
                Some(fctl_proto::ErrorCode::Validation),
                "{bad}"
            );
        }
    }
}
