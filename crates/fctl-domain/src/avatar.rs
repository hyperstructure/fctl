//! Avatars: the pictures people and workspaces upload, and the one a
//! person's Google account brings when they link it.

use crate::files;
use crate::validate::Result;
use fctl_proto::ApiError;

/// The largest avatar image, in bytes. The web app makes them 256 px square.
pub const MAX_BYTES: usize = 256 * 1000;
/// The widest and tallest avatar image, in pixels.
pub const MAX_SIDE: u32 = 1024;

/// Checks an avatar image and returns its media type: a PNG, JPEG, GIF or
/// WebP of at most [`MAX_BYTES`] and [`MAX_SIDE`] a side.
pub fn check(bytes: &[u8]) -> Result<&'static str> {
    if bytes.len() > MAX_BYTES {
        return Err(ApiError::validation(
            "avatar",
            format!(
                "Use an image of at most {}",
                files::human_size(MAX_BYTES as u64)
            ),
        ));
    }
    let info = files::sniff_image(bytes)
        .ok_or_else(|| ApiError::validation("avatar", "Use a PNG, JPEG, GIF or WebP image"))?;
    if info.width > MAX_SIDE || info.height > MAX_SIDE {
        return Err(ApiError::validation(
            "avatar",
            format!("Use an image of at most {MAX_SIDE}×{MAX_SIDE} pixels"),
        ));
    }
    Ok(info.media_type)
}

/// The path of a person's uploaded avatar, which changes with it. Their
/// `avatar_url` holds it, so everything that shows them shows it.
pub fn user_path(user_id: &str, updated_at: i64) -> String {
    format!("/api/v1/users/{user_id}/avatar?v={updated_at}")
}

/// Where to fetch a Google account's picture (its ID token's `picture`) at
/// the size the web app shows, 256 px square: only over HTTPS from Google's
/// servers for user content (`*.googleusercontent.com`), so that the
/// control plane fetches nothing else. Google sizes these pictures by the
/// options after the last path segment's `=` (`…=s96-c`).
pub fn google_picture(picture: &str) -> Option<String> {
    let rest = picture.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/')?;
    let host = host.to_ascii_lowercase();
    let google = host
        .strip_suffix(".googleusercontent.com")
        .is_some_and(|sub| {
            !sub.is_empty()
                && sub
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        });
    if !google
        || picture.len() > 2000
        || path
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '\\' | '?' | '#'))
    {
        return None;
    }
    let last = path.rsplit('/').next().unwrap_or_default();
    let path = match last.rsplit_once('=') {
        Some((name, _)) => format!("{}{name}=s256-c", &path[..path.len() - last.len()]),
        None if !last.is_empty() && !last.contains('.') => format!("{path}=s256-c"),
        None => path.to_owned(),
    };
    Some(format!("https://{host}/{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatars_are_small_images() {
        // A 1×1 PNG.
        let png =
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";
        assert_eq!(check(png).ok(), Some("image/png"));
        assert!(check(b"<svg xmlns='http://www.w3.org/2000/svg'/>").is_err());
        let mut big = png.to_vec();
        big.resize(MAX_BYTES + 1, 0);
        assert!(check(&big).is_err());
        // 2000 px wide.
        let wide =
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\x07\xd0\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";
        assert!(check(wide).is_err());
    }

    #[test]
    fn uploaded_avatars_change_address_with_them() {
        assert_eq!(
            user_path("usr_01", 1_700_000_000_000),
            "/api/v1/users/usr_01/avatar?v=1700000000000"
        );
    }

    #[test]
    fn google_pictures_come_from_google_at_256_px() {
        for (picture, fetched) in [
            (
                "https://lh3.googleusercontent.com/a/ACg8ocLKoO87oE1Ej-cp=s96-c",
                "https://lh3.googleusercontent.com/a/ACg8ocLKoO87oE1Ej-cp=s256-c",
            ),
            (
                "https://lh3.googleusercontent.com/a-/AOh14Gg3x=s96-c-k-no",
                "https://lh3.googleusercontent.com/a-/AOh14Gg3x=s256-c",
            ),
            (
                "https://LH5.GoogleUserContent.com/a/ACg8oc",
                "https://lh5.googleusercontent.com/a/ACg8oc=s256-c",
            ),
            // An older kind, sized in its path: as it is.
            (
                "https://lh3.googleusercontent.com/-Xd/AAAAAAAAAAI/s96-c/photo.jpg",
                "https://lh3.googleusercontent.com/-Xd/AAAAAAAAAAI/s96-c/photo.jpg",
            ),
        ] {
            assert_eq!(
                google_picture(picture).as_deref(),
                Some(fetched),
                "{picture}"
            );
        }
        for elsewhere in [
            "http://lh3.googleusercontent.com/a/x=s96-c",
            "https://googleusercontent.com/a/x=s96-c",
            "https://lh3.googleusercontent.com.example.com/a/x=s96-c",
            "https://example.com/lh3.googleusercontent.com/a/x",
            "https://someone@lh3.googleusercontent.com/a/x",
            "https://lh3.googleusercontent.com:8443/a/x",
            "https://lh3.googleusercontent.com",
            "https://lh3.googleusercontent.com/a/x?sz=50",
            "https://lh3.googleusercontent.com/a/x y",
            "https://lh3.googleusercontent.com/a\\@example.com/x",
        ] {
            assert_eq!(google_picture(elsewhere), None, "{elsewhere}");
        }
    }
}
