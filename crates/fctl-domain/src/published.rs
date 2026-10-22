//! Screenshots agents publish on the control plane (`fctl publish`), so that
//! pull requests can show them: GitHub takes no uploads from a session's
//! GitHub App token. Which files, how large, for how long, and their links.

use crate::files::{self, ImageInfo};
use crate::random_bytes;
use crate::tokens::base62;
use fctl_proto::ApiError;
use fctl_proto::event::PublishedFile;
use fctl_proto::session::ProjectSettings;

/// Largest screenshot accepted (5 MB): GitHub's image proxy, which fetches
/// the images in pull requests, refuses larger ones.
pub const MAX_BYTES: u64 = 5 * 1000 * 1000;
/// Days a link works unless the project chooses otherwise.
pub const DEFAULT_DAYS: u32 = 30;
/// The shortest a project may choose: 24 hours.
pub const MIN_DAYS: u32 = 1;
/// The longest a project may choose.
pub const MAX_DAYS: u32 = 60;
/// Most screenshots one session may publish.
pub const MAX_PER_SESSION: u32 = 100;
/// Where links start, after the instance's origin.
pub const PATH_PREFIX: &str = "/api/v1/published/";
/// The prefix of a published screenshot's id (`pub_…`), which only the
/// control plane uses: links name it by token.
pub const ID_PREFIX: &str = "pub";

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Base62 digits of a token: 256 bits.
const TOKEN_LEN: usize = 43;
/// Longest name in a link, in bytes.
const MAX_URL_NAME: usize = 100;

/// Whether a project's agents may publish screenshots, and its published
/// screenshots are online: its admins allow it for the instance, and the
/// project turned it on.
pub fn allowed(instance: bool, settings: &ProjectSettings) -> bool {
    instance && settings.publish_screenshots
}

/// How many days a project's links work.
pub fn days(settings: &ProjectSettings) -> u32 {
    settings
        .screenshot_days
        .unwrap_or(DEFAULT_DAYS)
        .clamp(MIN_DAYS, MAX_DAYS)
}

/// Whether a project may choose `days`.
pub fn valid_days(days: u32) -> bool {
    (MIN_DAYS..=MAX_DAYS).contains(&days)
}

/// When a screenshot published at `now` stops working.
pub fn expires_at(now: i64, days: u32) -> i64 {
    now + i64::from(days) * DAY_MS
}

/// What can be published: a PNG, JPEG, GIF or WebP image, as its bytes
/// say, within [`MAX_BYTES`]. Those are the images GitHub shows in pull
/// requests; anything a browser could run (SVG, HTML) is refused.
pub fn check(bytes: &[u8]) -> Result<ImageInfo, ApiError> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err(ApiError::validation(
            "file",
            format!(
                "Screenshots can be at most {}: GitHub doesn't show larger images",
                files::human_size(MAX_BYTES)
            ),
        ));
    }
    files::sniff_image(bytes).ok_or_else(|| {
        ApiError::validation(
            "file",
            "Only PNG, JPEG, GIF and WebP images can be published",
        )
    })
}

/// A new link token: 256 random bits in base62. It has no type prefix,
/// unlike secrets: it goes in links meant to be shared.
pub fn new_token() -> String {
    let bytes: [u8; 32] = random_bytes();
    base62(&bytes)
}

/// Whether a link's segment has the shape of a token.
pub fn is_token(s: &str) -> bool {
    s.len() == TOKEN_LEN && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The token in a link (or just the token), if there is one.
pub fn token_in(link: &str) -> Option<&str> {
    let link = link.trim();
    let rest = match link.find(PATH_PREFIX) {
        Some(i) => &link[i + PATH_PREFIX.len()..],
        None => link,
    };
    let token = rest.split(['/', '?', '#']).next()?;
    is_token(token).then_some(token)
}

/// Whether `url` is the link of a screenshot that this instance (its
/// origin, `public_url`) published, as [`path`] makes them:
/// `<origin>/api/v1/published/<token>/<name>`, nothing more. A notice that
/// says the agent published one comes from the sandbox, which can pose as
/// the agent, so its link is held to that.
pub fn is_link(url: &str, public_url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix(public_url)
        .and_then(|r| r.strip_prefix(PATH_PREFIX))
    else {
        return false;
    };
    let Some((token, name)) = rest.split_once('/') else {
        return false;
    };
    is_token(token)
        && !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// The file's name as the link's last segment, which only people read: a
/// readable name with the image's extension, in characters links keep as
/// they are.
pub fn url_name(name: &str, media_type: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        let c = if c.is_ascii_alphanumeric() || "._-".contains(c) {
            c
        } else {
            '-'
        };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    let mut out = out.trim_matches(['-', '.']).to_owned();
    out.truncate(MAX_URL_NAME);
    let out = out.trim_end_matches(['-', '.']);
    let ext = extension(media_type);
    let lower = out.to_ascii_lowercase();
    let has_ext = lower
        .rsplit_once('.')
        .is_some_and(|(_, e)| e == ext || (ext == "jpg" && e == "jpeg"));
    match (out.is_empty(), has_ext) {
        (true, _) => format!("screenshot.{ext}"),
        (false, true) => out.to_owned(),
        (false, false) => format!("{out}.{ext}"),
    }
}

fn extension(media_type: &str) -> &'static str {
    match media_type {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    }
}

/// A link's path: `/api/v1/published/<token>/<name>`.
pub fn path(token: &str, name: &str, media_type: &str) -> String {
    format!("{PATH_PREFIX}{token}/{}", url_name(name, media_type))
}

/// Markdown that shows the screenshot, its name as the alternative text.
pub fn markdown(file: &PublishedFile) -> String {
    let alt: String = file
        .name
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | '\\' | '\n' | '\r'))
        .collect();
    format!("![{}]({})", alt.trim(), file.url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        b
    }

    #[test]
    fn needs_the_instance_and_the_project() {
        let on = ProjectSettings {
            publish_screenshots: true,
            ..Default::default()
        };
        assert!(allowed(true, &on));
        assert!(!allowed(false, &on));
        assert!(!allowed(true, &ProjectSettings::default()));
    }

    #[test]
    fn links_work_for_thirty_days_unless_the_project_chooses() {
        let mut s = ProjectSettings::default();
        assert_eq!(days(&s), 30);
        s.screenshot_days = Some(1);
        assert_eq!(days(&s), 1);
        s.screenshot_days = Some(60);
        assert_eq!(days(&s), 60);
        // Stored before validation existed, or by hand: within the bounds.
        s.screenshot_days = Some(0);
        assert_eq!(days(&s), 1);
        s.screenshot_days = Some(365);
        assert_eq!(days(&s), 60);
        assert!(valid_days(1) && valid_days(60));
        assert!(!valid_days(0) && !valid_days(61));
        assert_eq!(expires_at(1_000, 1), 1_000 + 86_400_000);
    }

    #[test]
    fn publishes_only_images_github_shows() {
        let info = check(&png(1440, 900)).map(|i| (i.media_type, i.width, i.height));
        assert_eq!(info.ok(), Some(("image/png", 1440, 900)));
        let svg = check(b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script/></svg>");
        assert_eq!(svg.err().and_then(|e| e.field).as_deref(), Some("file"));
        assert!(check(b"<html><script>alert(1)</script></html>").is_err());
        let mut big = png(4000, 3000);
        big.resize(MAX_BYTES as usize + 1, 0);
        assert!(
            check(&big)
                .err()
                .is_some_and(|e| e.message.contains("5.0 MB"))
        );
        big.truncate(MAX_BYTES as usize);
        assert!(check(&big).is_ok());
    }

    #[test]
    fn tokens_are_long_and_random() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 43);
        assert!(is_token(&a));
        assert_ne!(a, b);
        assert!(!is_token("short"));
        assert!(!is_token(&format!("{}-", &a[..42])));
        assert!(!is_token(&format!("{a}0")));
    }

    #[test]
    fn links_are_held_to_the_shape_the_control_plane_makes() {
        let token = new_token();
        let app = "https://app.fctl.sh";
        let link = format!("{app}{}", path(&token, "after shot.png", "image/png"));
        assert!(is_link(&link, app));
        for bad in [
            // Another instance's, or another page of this one.
            format!("https://evil.example{PATH_PREFIX}{token}/after.png"),
            format!("{app}.evil.example{PATH_PREFIX}{token}/after.png"),
            format!("{app}{PATH_PREFIX}../../github/install?workspace=acme"),
            format!("{app}{PATH_PREFIX}{token}/../../../auth/google/start"),
            format!("{app}{PATH_PREFIX}{token}/after.png?next=/device"),
            format!("{app}{PATH_PREFIX}{token}"),
            format!("{app}{PATH_PREFIX}short/after.png"),
            format!("{app}{PATH_PREFIX}{token}/.."),
        ] {
            assert!(!is_link(&bad, app), "{bad}");
        }
    }

    #[test]
    fn finds_the_token_in_a_link() {
        let token = new_token();
        let link = format!("https://app.fctl.sh/api/v1/published/{token}/after.png");
        assert_eq!(token_in(&link), Some(token.as_str()));
        assert_eq!(token_in(&format!(" {token} ")), Some(token.as_str()));
        assert_eq!(
            token_in(&format!("http://127.0.0.1:8787{PATH_PREFIX}{token}?x=1")),
            Some(token.as_str())
        );
        assert_eq!(token_in("https://app.fctl.sh/s/ses_1"), None);
        assert_eq!(
            token_in("https://app.fctl.sh/api/v1/published/abc/x.png"),
            None
        );
    }

    #[test]
    fn names_links_readably() {
        assert_eq!(url_name("after-light.png", "image/png"), "after-light.png");
        assert_eq!(
            url_name("Skärmavbild 2026-10-18 kl. 14.05.png", "image/png"),
            "Sk-rmavbild-2026-10-18-kl.-14.05.png"
        );
        assert_eq!(url_name("photo.JPEG", "image/jpeg"), "photo.JPEG");
        assert_eq!(url_name("shot", "image/webp"), "shot.webp");
        assert_eq!(url_name("notes.txt", "image/gif"), "notes.txt.gif");
        assert_eq!(url_name("???", "image/png"), "screenshot.png");
        assert_eq!(url_name("../../x.png", "image/png"), "x.png");
        let long = url_name(&format!("{}.png", "a".repeat(300)), "image/png");
        assert!(long.len() <= MAX_URL_NAME + 4, "{}", long.len());
        assert!(long.ends_with(".png"));
        assert_eq!(
            path("TOKEN", "after light.png", "image/png"),
            "/api/v1/published/TOKEN/after-light.png"
        );
    }

    #[test]
    fn markdown_shows_the_image() {
        let file = PublishedFile {
            url: "https://app.fctl.sh/api/v1/published/abc/after.png".into(),
            name: "after [light].png".into(),
            media_type: "image/png".into(),
            size: 10,
            width: None,
            height: None,
            expires_at: 0,
        };
        assert_eq!(
            markdown(&file),
            "![after light.png](https://app.fctl.sh/api/v1/published/abc/after.png)"
        );
    }
}
