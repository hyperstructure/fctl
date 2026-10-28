//! Input validation. Every function returns the cleaned value or an
//! [`ApiError`] naming the field, so handlers can return it as is.

use crate::webpush;
use fctl_proto::session::ImageBuild;
use fctl_proto::{ApiError, AttachmentId};

pub type Result<T> = std::result::Result<T, ApiError>;

/// Trims and lowercases an email address and checks its shape.
pub fn email(input: &str) -> Result<String> {
    let e = input.trim().to_lowercase();
    let bad = || ApiError::validation("email", "Enter a valid email address");
    if e.len() > 254 || e.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad());
    }
    let (local, domain) = e.split_once('@').ok_or_else(bad)?;
    if local.is_empty() || local.len() > 64 || domain.contains('@') {
        return Err(bad());
    }
    if !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || domain.contains("..")
        || !domain
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '.')
    {
        return Err(bad());
    }
    Ok(e)
}

/// The address commits credit someone with (`Co-authored-by: Name
/// <address>`), or `None` when empty. It keeps its case, since GitHub
/// matches it against its accounts, and its local part is a plain
/// dot-atom, so it can't break the trailer.
pub fn commit_email(input: &str) -> Result<Option<String>> {
    let e = input.trim();
    if e.is_empty() {
        return Ok(None);
    }
    let bad = || ApiError::validation("commit_email", "Enter a valid email address");
    email(e).map_err(|_| bad())?;
    let local = e.split('@').next().unwrap_or_default();
    if local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || !local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~.".contains(c))
    {
        return Err(bad());
    }
    Ok(Some(e.to_owned()))
}

/// Passwords: 10–256 characters, not trivially guessable.
pub fn password(pw: &str, email: Option<&str>) -> Result<()> {
    let n = pw.chars().count();
    if n < 10 {
        return Err(ApiError::validation(
            "password",
            "Use at least 10 characters",
        ));
    }
    if n > 256 {
        return Err(ApiError::validation(
            "password",
            "Use at most 256 characters",
        ));
    }
    let first = pw.chars().next();
    if pw.chars().all(|c| Some(c) == first) {
        return Err(ApiError::validation(
            "password",
            "Use more than one distinct character",
        ));
    }
    if let Some(email) = email {
        let lower = pw.to_lowercase();
        let local = email.split('@').next().unwrap_or_default();
        if lower == email || (local.len() >= 4 && lower.contains(local)) {
            return Err(ApiError::validation(
                "password",
                "Don't base the password on your email address",
            ));
        }
    }
    const COMMON: &[&str] = &[
        "password123",
        "1234567890",
        "qwertyuiop",
        "password1234",
        "letmein123",
        "iloveyou123",
        "0123456789",
        "1q2w3e4r5t",
        "qwerty1234",
        "administrator",
    ];
    if COMMON.contains(&pw.to_lowercase().as_str()) {
        return Err(ApiError::validation(
            "password",
            "This password is too common",
        ));
    }
    Ok(())
}

/// A display name: 1–80 characters, without control characters or ones
/// that turn the text around, which names show nowhere: a device asking
/// to sign in, say, could otherwise read as another.
pub fn name(field: &str, input: &str) -> Result<String> {
    let s = collapse_spaces(&plain(input));
    if s.is_empty() {
        return Err(ApiError::validation(field, "Enter a name"));
    }
    if s.chars().count() > 80 {
        return Err(ApiError::validation(field, "Use at most 80 characters"));
    }
    Ok(s)
}

/// A person's names, cleaned. `full` is what everyone sees; `given` (first
/// names, all of them: "Carl Oscar") is what greetings use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonName {
    pub full: String,
    pub given: Option<String>,
    pub family: Option<String>,
}

/// From first names and an optional last name, or, when `given` is absent
/// (older clients), from one full name whose parts are then unknown.
pub fn person_name(
    given: Option<&str>,
    family: Option<&str>,
    full: Option<&str>,
) -> Result<PersonName> {
    let Some(given) = given else {
        return Ok(PersonName {
            full: name("name", full.unwrap_or_default())?,
            given: None,
            family: None,
        });
    };
    let given = name("given_name", given)
        .map_err(|_| ApiError::validation("given_name", "Enter your first name (all of them)"))?;
    let family = collapse_spaces(&plain(family.unwrap_or_default()));
    if family.chars().count() > 80 {
        return Err(ApiError::validation(
            "family_name",
            "Use at most 80 characters",
        ));
    }
    let family = (!family.is_empty()).then_some(family);
    let full = match &family {
        Some(f) => format!("{given} {f}"),
        None => given.clone(),
    };
    Ok(PersonName {
        full,
        given: Some(given),
        family,
    })
}

/// What someone asked to be called (`User.preferred_name`): a [`name`], or
/// `None` for an empty one, which leaves them their first names.
pub fn preferred_name(input: &str) -> Result<Option<String>> {
    if collapse_spaces(&plain(input)).is_empty() {
        return Ok(None);
    }
    name("preferred_name", input).map(Some)
}

/// What agents call someone: the name they chose, or else their first
/// names, all of them ("Carl Oscar"). `None` when fctl knows neither, as
/// for an account an older client created with only a full name.
pub fn called(preferred: Option<&str>, given: Option<&str>) -> Option<String> {
    [preferred, given]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|n| !n.is_empty())
        .map(str::to_owned)
}

/// Optional free text (descriptions): at most `max` characters.
pub fn text(field: &str, input: &str, max: usize) -> Result<String> {
    let s = input.trim().to_owned();
    if s.chars().count() > max {
        return Err(ApiError::validation(
            field,
            format!("Use at most {max} characters"),
        ));
    }
    if s.chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t' && c != '\r')
    {
        return Err(ApiError::validation(field, "Remove control characters"));
    }
    Ok(s)
}

/// A session title: a [`name`] without control characters. Agents write
/// titles too, and terminals print them.
pub fn session_title(input: &str) -> Result<String> {
    if input.chars().any(|c| c.is_control() && !c.is_whitespace()) {
        return Err(ApiError::validation("title", "Remove control characters"));
    }
    name("title", input)
}

/// What a session is about: one paragraph of at most 500 characters (runs
/// of whitespace, newlines too, become one space). Empty clears it.
pub fn session_description(input: &str) -> Result<Option<String>> {
    let s = collapse_spaces(input);
    if s.chars().count() > 500 {
        return Err(ApiError::validation(
            "description",
            "Use at most 500 characters",
        ));
    }
    if s.chars().any(char::is_control) {
        return Err(ApiError::validation(
            "description",
            "Remove control characters",
        ));
    }
    Ok((!s.is_empty()).then_some(s))
}

fn collapse_spaces(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Text without control characters, but for spaces, and without the ones
/// that turn it around (`crate::files::bidi_control`).
fn plain(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_whitespace() || !(c.is_control() || crate::files::bidi_control(*c)))
        .collect()
}

/// Slugs that would collide with routes: fctl's own, which no workspace or
/// project takes. Instance admins reserve more (`crate::slugs`).
pub const RESERVED_SLUGS: &[&str] = &[
    "api", "admin", "new", "settings", "login", "logout", "signup", "setup", "device", "invite",
    "s", "p", "projects", "sessions", "runners", "assets", "static", "app", "help", "docs",
];

/// A URL slug: 2–48 of `a-z 0-9 -`, no leading, trailing or double dashes,
/// and none of fctl's own ([`RESERVED_SLUGS`]).
pub fn slug(input: &str) -> Result<String> {
    let s = slug_shape(input)?;
    if RESERVED_SLUGS.contains(&s.as_str()) {
        return Err(ApiError::validation("slug", "This name is reserved"));
    }
    Ok(s)
}

/// A URL slug's shape alone, as [`slug`] checks it: fctl's own pass.
pub fn slug_shape(input: &str) -> Result<String> {
    let s = input.trim().to_lowercase();
    let bad = |m: &str| ApiError::validation("slug", m);
    if s.len() < 2 || s.len() > 48 {
        return Err(bad("Use 2–48 characters"));
    }
    if !s
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(bad("Use lowercase letters, digits and dashes"));
    }
    if s.starts_with('-') || s.ends_with('-') || s.contains("--") {
        return Err(bad("Dashes go between words"));
    }
    Ok(s)
}

/// A git branch name, following `git check-ref-format --branch`.
pub fn branch(field: &str, input: &str) -> Result<String> {
    let s = input.trim();
    let bad = || ApiError::validation(field, "Not a valid branch name");
    if s.is_empty() || s.len() > 200 {
        return Err(bad());
    }
    let forbidden = |c: char| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c);
    if s.chars().any(forbidden)
        || s.starts_with('-')
        || s.starts_with('/')
        || s.ends_with('/')
        || s.ends_with('.')
        || s.ends_with(".lock")
        || s.contains("..")
        || s.contains("//")
        || s.contains("@{")
        || s == "@"
        || s.split('/')
            .any(|part| part.starts_with('.') || part.ends_with(".lock"))
    {
        return Err(bad());
    }
    Ok(s.to_owned())
}

/// Environment variables people may not set: they would switch Claude to
/// API billing or redirect its traffic, or break the sandbox.
pub fn is_reserved_env(name: &str) -> bool {
    const EXACT: &[&str] = &[
        "HOME",
        "PATH",
        "USER",
        "SHELL",
        "PWD",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "AWS_BEARER_TOKEN_BEDROCK",
    ];
    const PREFIXES: &[&str] = &["ANTHROPIC_", "CLAUDE_CODE_OAUTH", "CLAUDE_CONFIG", "FCTL_"];
    EXACT.contains(&name) || PREFIXES.iter().any(|p| name.starts_with(p))
}

/// An environment variable name: `[A-Z_][A-Z0-9_]*`, at most 128 characters.
pub fn env_name(field: &str, input: &str) -> Result<String> {
    let s = input.trim();
    let ok = !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && !s.as_bytes()[0].is_ascii_digit();
    if !ok {
        return Err(ApiError::validation(
            field,
            "Use uppercase letters, digits and underscores (like API_URL)",
        ));
    }
    if is_reserved_env(s) {
        return Err(ApiError::validation(
            field,
            format!("{s} is managed by fctl and cannot be set"),
        ));
    }
    Ok(s.to_owned())
}

/// A prompt or message: not empty, at most 100 000 characters.
pub fn message(field: &str, input: &str) -> Result<String> {
    let s = input.trim();
    if s.is_empty() {
        return Err(ApiError::validation(field, "Write something first"));
    }
    if s.chars().count() > 100_000 {
        return Err(ApiError::validation(
            field,
            "Messages can be at most 100 000 characters",
        ));
    }
    Ok(s.to_owned())
}

/// A message sent with `files` attachments: like [`message`], but it may be
/// empty when files go with it.
pub fn message_with_files(field: &str, input: &str, files: usize) -> Result<String> {
    if files > 0 && input.trim().is_empty() {
        return Ok(String::new());
    }
    message(field, input)
}

/// The attachments of one message: well-formed ids, each once, at most
/// [`crate::files::MAX_FILES_PER_MESSAGE`].
pub fn attachment_ids(ids: &[AttachmentId]) -> Result<Vec<AttachmentId>> {
    let mut out: Vec<AttachmentId> = Vec::with_capacity(ids.len());
    for id in ids {
        if !AttachmentId::is_valid(id.as_str()) {
            return Err(ApiError::validation(
                "attachments",
                "Unknown attachment; upload the file again",
            ));
        }
        if !out.contains(id) {
            out.push(id.clone());
        }
    }
    if out.len() > crate::files::MAX_FILES_PER_MESSAGE {
        return Err(ApiError::validation(
            "attachments",
            format!(
                "Attach at most {} files to one message",
                crate::files::MAX_FILES_PER_MESSAGE
            ),
        ));
    }
    Ok(out)
}

/// A Claude model alias or id.
pub fn model(input: &str) -> Result<String> {
    let s = input.trim();
    if s.is_empty()
        || s.len() > 100
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._[]:".contains(&b))
    {
        return Err(ApiError::validation("model", "Not a valid model name"));
    }
    Ok(s.to_owned())
}

/// A Claude effort level.
pub fn effort(input: &str) -> Result<String> {
    let s = input.trim().to_lowercase();
    match s.as_str() {
        "low" | "medium" | "high" | "xhigh" | "max" => Ok(s),
        _ => Err(ApiError::validation(
            "effort",
            "Use low, medium, high, xhigh or max",
        )),
    }
}

/// An OCI image reference.
pub fn image(input: &str) -> Result<String> {
    let s = input.trim();
    if s.is_empty()
        || s.len() > 255
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._/:@".contains(&b))
        || crate::image::ImageRef::parse(s).is_none()
    {
        return Err(ApiError::validation("image", "Not a valid image reference"));
    }
    Ok(s.to_owned())
}

/// A sandbox image built from the project's repository: a file in it, and
/// for a Dockerfile, the directory it builds in (`None` for the root).
pub fn image_build(build: &ImageBuild) -> Result<ImageBuild> {
    const FIELD: &str = "settings.image_build";
    let file = repo_path(&build.file)
        .filter(|f| !f.is_empty())
        .ok_or_else(|| {
            ApiError::validation(
                FIELD,
                "Name a Dockerfile or a devcontainer.json by its path in the repository",
            )
        })?;
    let context = match build.context.as_deref() {
        None => None,
        Some(c) => Some(repo_path(c).ok_or_else(|| {
            ApiError::validation(
                FIELD,
                "Name the build's directory by its path in the repository",
            )
        })?),
    }
    .filter(|c| !c.is_empty());
    let build = ImageBuild { file, context };
    if build.is_devcontainer() && build.context.is_some() {
        return Err(ApiError::validation(
            FIELD,
            "A devcontainer.json says where it builds; leave the directory empty",
        ));
    }
    Ok(build)
}

/// A relative path in a repository, without `./` at its start or a slash at
/// its end: `""` for the root. `None` when it leaves the repository or isn't
/// a path.
fn repo_path(input: &str) -> Option<String> {
    let mut s = input.trim();
    while let Some(rest) = s.strip_prefix("./") {
        s = rest;
    }
    let s = s.trim_end_matches('/');
    if s == "." || s.is_empty() {
        return Some(String::new());
    }
    let ok = s.len() <= 255
        && !s.starts_with('/')
        && !s.chars().any(|c| c.is_control() || c == '\\')
        && s.split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..");
    ok.then(|| s.to_owned())
}

/// A push subscription's endpoint: an https URL on a named host, at the
/// browser's push service. With `local` (development), plain http and local
/// hosts too.
pub fn push_endpoint(input: &str, local: bool) -> Result<String> {
    let bad = || ApiError::validation("endpoint", "Not a push subscription endpoint");
    let s = input.trim();
    if s.len() > 2048 || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad());
    }
    let origin = webpush::audience(s).ok_or_else(bad)?;
    let (scheme, host) = origin.split_once("://").ok_or_else(bad)?;
    if !host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-.:[]".contains(&b))
    {
        return Err(bad());
    }
    // Push services listen on 443: another port would point the control
    // plane's requests at whatever else listens on a host.
    let port = host.rsplit_once(':').map(|(_, port)| port);
    if !local
        && (scheme != "https"
            || !public_name(&webpush::service(s))
            || port.is_some_and(|p| p != "443"))
    {
        return Err(ApiError::validation(
            "endpoint",
            "Push endpoints are https:// addresses of a push service",
        ));
    }
    Ok(s.to_owned())
}

/// A host name on the internet: dot-separated labels of letters, digits and
/// hyphens, ending in a top-level domain that starts with a letter. That
/// leaves out IP addresses in every notation (`127.1`, `0x7f.0.0.1`, `[::1]`),
/// `localhost`, and names with a trailing dot.
fn public_name(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    let ok = |l: &&str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    labels.len() >= 2
        && labels.iter().all(ok)
        && labels
            .last()
            .is_some_and(|tld| tld.starts_with(|c: char| c.is_ascii_alphabetic()))
        && !host.eq_ignore_ascii_case("localhost")
        && !host.to_ascii_lowercase().ends_with(".localhost")
}

/// A push subscription's keys, as base64url without padding: the browser's
/// P-256 public key (uncompressed) and its 16-byte authentication secret.
pub fn push_keys(p256dh: &str, auth: &str) -> Result<(String, String)> {
    let key = webpush::decode(p256dh)
        .filter(|k| k.len() == webpush::PUBLIC_KEY_LEN && k.first() == Some(&4))
        .ok_or_else(|| ApiError::validation("keys.p256dh", "Not a P-256 public key"))?;
    let secret = webpush::decode(auth)
        .filter(|a| a.len() == webpush::AUTH_LEN)
        .ok_or_else(|| ApiError::validation("keys.auth", "Not a push authentication secret"))?;
    Ok((webpush::encode(&key), webpush::encode(&secret)))
}

/// What to call a browser in Settings: one line of at most 80 characters.
/// Empty is no name.
pub fn device_name(input: &str) -> Result<Option<String>> {
    let s = collapse_spaces(&plain(input));
    if s.chars().count() > 80 {
        return Err(ApiError::validation("name", "Use at most 80 characters"));
    }
    Ok((!s.is_empty()).then_some(s))
}

/// Whether `path` is a path of the app, for redirects after a sign-in: it
/// starts with one `/`. Browsers read `\` as `/` and drop tabs and
/// newlines, so `/\evil.example` and `/<tab>/evil.example` would leave the
/// app: paths with a backslash, whitespace or control character aren't.
pub fn is_app_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path
            .chars()
            .any(|c| c == '\\' || c.is_whitespace() || c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_paths() {
        for ok in [
            "/",
            "/settings",
            "/s/ses_1?x=1#y",
            "/invite",
            "/.//x",
            "/%5Cx",
        ] {
            assert!(is_app_path(ok), "{ok}");
        }
        for bad in [
            "",
            "settings",
            "//evil.example",
            "/\\evil.example",
            "/\t/evil.example",
            "/\n/evil.example",
            "/\r/evil.example",
            "/ /evil.example",
            "/\u{3000}/evil.example",
            "https://evil.example",
            "javascript:alert(1)",
        ] {
            assert!(!is_app_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn commit_emails() {
        assert_eq!(commit_email(" ").ok(), Some(None));
        // GitHub's noreply form, case kept.
        assert_eq!(
            commit_email(" 1234+Octo-Cat@users.noreply.github.com ")
                .ok()
                .flatten()
                .as_deref(),
            Some("1234+Octo-Cat@users.noreply.github.com")
        );
        assert!(commit_email("ada.lovelace@example.com").is_ok());
        for bad in [
            "ada",
            "ada@example",
            "Ada <ada@example.com>",
            "ada>@example.com",
            "ada@example.com>",
            "ada\n@example.com",
            "\"ada\"@example.com",
            ".ada@example.com",
            "ada..l@example.com",
            "ada@@example.com",
        ] {
            let err = commit_email(bad).err();
            assert_eq!(
                err.and_then(|e| e.field).as_deref(),
                Some("commit_email"),
                "{bad:?}"
            );
        }
        assert!(commit_email(&format!("{}@example.com", "a".repeat(65))).is_err());
    }

    #[test]
    fn push_subscriptions() {
        let fcm = "https://fcm.googleapis.com/fcm/send/dQw4w9WgXcQ:APA91bH";
        assert_eq!(push_endpoint(fcm, false).ok().as_deref(), Some(fcm));
        assert!(push_endpoint("https://web.push.apple.com/QGx", false).is_ok());
        assert!(push_endpoint("http://fcm.googleapis.com/fcm/send/x", false).is_err());
        assert!(push_endpoint("https://localhost/push", false).is_err());
        assert!(push_endpoint("https://10.0.0.8/push", false).is_err());
        assert!(push_endpoint("https://[::1]/push", false).is_err());
        assert!(push_endpoint("https://push.localhost/x", false).is_err());
        // IPv4 in other notations, and names that resolve here.
        for host in [
            "127.1",
            "0x7f.0.0.1",
            "0177.0.0.1",
            "localhost.",
            "fcm.googleapis.com.",
        ] {
            assert!(
                push_endpoint(&format!("https://{host}/push"), false).is_err(),
                "{host}"
            );
        }
        assert!(push_endpoint("https://wns2-par02p.notify.windows.com/w/?token=x", false).is_ok());
        assert!(push_endpoint("https://push.example.xn--p1ai/x", false).is_ok());
        assert!(push_endpoint("https://user@evil.example/x", false).is_err());
        assert!(push_endpoint("https://fcm.googleapis.com/a b", false).is_err());
        // Push services listen on 443, and nothing else is theirs to reach.
        assert!(push_endpoint("https://fcm.googleapis.com:443/fcm/send/x", false).is_ok());
        assert!(push_endpoint("https://evil.example:9200/_search", false).is_err());
        assert!(push_endpoint("https://evil.example:80/x", false).is_err());
        assert!(push_endpoint("javascript:alert(1)", true).is_err());
        // Development: a push service on this machine.
        assert!(push_endpoint("http://127.0.0.1:9000/push/1", true).is_ok());

        let key = webpush::encode(&[4; 65]);
        let auth = webpush::encode(&[7; 16]);
        assert_eq!(
            push_keys(&format!("{key}="), &auth).ok(),
            Some((key.clone(), auth.clone()))
        );
        assert!(push_keys(&webpush::encode(&[2; 65]), &auth).is_err());
        assert!(push_keys(&key, &webpush::encode(&[7; 15])).is_err());
        assert!(push_keys("not base64!", &auth).is_err());

        assert_eq!(
            device_name("  Chrome   on macOS ")
                .ok()
                .flatten()
                .as_deref(),
            Some("Chrome on macOS")
        );
        assert_eq!(device_name(" ").ok(), Some(None));
        assert!(device_name(&"x".repeat(81)).is_err());
        assert_eq!(
            device_name("Chrome\u{202E}SOcam no\u{7}")
                .ok()
                .flatten()
                .as_deref(),
            Some("ChromeSOcam no")
        );
    }

    #[test]
    fn names_show_as_they_read() {
        assert_eq!(
            name("name", "fctl on Ada's\u{202E}kooBcaM\u{2069}")
                .ok()
                .as_deref(),
            Some("fctl on Ada'skooBcaM")
        );
        assert_eq!(
            name("name", "Ada\u{1b}[2J\nLovelace\u{7}").ok().as_deref(),
            Some("Ada[2J Lovelace")
        );
        assert!(name("name", "\u{200F}\u{7}").is_err());
        let p = person_name(Some("Ada\u{202D}"), Some("Love\u{0}lace"), None)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(p.full, "Ada Lovelace");
    }

    #[test]
    fn messages_with_files() {
        assert_eq!(
            message_with_files("text", "  ", 1).ok(),
            Some(String::new())
        );
        assert!(message_with_files("text", "  ", 0).is_err());
        assert_eq!(
            message_with_files("text", " hi ", 2).ok(),
            Some("hi".into())
        );
        let id = |s: &str| AttachmentId(s.into());
        let ids = attachment_ids(&[id("att_1"), id("att_2"), id("att_1")]);
        assert_eq!(ids.ok(), Some(vec![id("att_1"), id("att_2")]));
        assert!(attachment_ids(&[id("ses_1")]).is_err());
        assert!(attachment_ids(&[id("att_../x")]).is_err());
        let many: Vec<AttachmentId> = (0..11).map(|i| id(&format!("att_{i}"))).collect();
        assert!(attachment_ids(&many).is_err());
        assert!(attachment_ids(&many[..10]).is_ok());
    }

    #[test]
    fn session_titles() {
        assert_eq!(
            session_title("  Fix the   flaky login test ")
                .ok()
                .as_deref(),
            Some("Fix the flaky login test")
        );
        assert!(session_title(" ").is_err());
        assert!(session_title(&"x".repeat(81)).is_err());
        let escape = session_title("Fix \u{1b}[31mlogin").map_err(|e| e.message);
        assert_eq!(escape, Err("Remove control characters".into()));
    }

    #[test]
    fn session_descriptions() {
        assert_eq!(
            session_description("Retries uploads.\n\nAdds tests.").ok(),
            Some(Some("Retries uploads. Adds tests.".into()))
        );
        assert_eq!(session_description("  ").ok(), Some(None));
        assert_eq!(
            session_description(&"x".repeat(501)).map_err(|e| e.field),
            Err(Some("description".into()))
        );
        assert!(session_description("bell \u{7}").is_err());
    }

    #[test]
    fn person_names() {
        let n = person_name(Some(" Carl  Oscar "), Some("Aaro"), None)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(n.full, "Carl Oscar Aaro");
        assert_eq!(n.given.as_deref(), Some("Carl Oscar"));
        assert_eq!(n.family.as_deref(), Some("Aaro"));
        let mononym =
            person_name(Some("Cher"), Some("  "), None).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!((mononym.full.as_str(), mononym.family), ("Cher", None));
        let legacy =
            person_name(None, None, Some("Ada Lovelace")).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!((legacy.full.as_str(), legacy.given), ("Ada Lovelace", None));
        assert_eq!(
            person_name(Some(" "), Some("Aaro"), None).map_err(|e| e.field),
            Err(Some("given_name".into()))
        );
        assert!(person_name(None, None, None).is_err());
        assert!(person_name(Some("Ada"), Some(&"x".repeat(81)), None).is_err());
    }

    #[test]
    fn preferred_names() {
        assert_eq!(
            preferred_name("  Carl \n Oscar ").ok(),
            Some(Some("Carl Oscar".into()))
        );
        assert_eq!(preferred_name(" \u{202e} ").ok(), Some(None));
        assert_eq!(
            preferred_name(&"x".repeat(81)).map_err(|e| e.field),
            Err(Some("preferred_name".into()))
        );
    }

    #[test]
    fn agents_call_people_what_they_chose_or_all_their_first_names() {
        assert_eq!(called(Some("Bob"), Some("Robert")).as_deref(), Some("Bob"));
        assert_eq!(
            called(None, Some("Carl Oscar")).as_deref(),
            Some("Carl Oscar")
        );
        assert_eq!(
            called(Some(" "), Some("Carl Oscar")).as_deref(),
            Some("Carl Oscar")
        );
        assert_eq!(called(None, None), None);
    }

    #[test]
    fn emails() {
        assert_eq!(
            email("  Ada@Example.COM ").ok().as_deref(),
            Some("ada@example.com")
        );
        for bad in [
            "",
            "ada",
            "ada@",
            "@x.com",
            "a b@x.com",
            "ada@x",
            "ada@x..com",
            "a@@x.com",
        ] {
            assert!(email(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn passwords() {
        assert!(password("correct horse battery", None).is_ok());
        assert!(password("short", None).is_err());
        assert!(password("aaaaaaaaaaaa", None).is_err());
        assert!(password("password123", None).is_err());
        assert!(password("ada.lovelace!!", Some("ada.lovelace@x.com")).is_err());
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("My-App").ok().as_deref(), Some("my-app"));
        for bad in ["a", "-a", "a-", "a--b", "a_b", "api", "über"] {
            assert!(slug(bad).is_err(), "{bad}");
        }
        // fctl's own have a slug's shape.
        assert_eq!(slug_shape(" Api ").ok().as_deref(), Some("api"));
        for bad in ["a", "-a", "a-", "a--b", "a_b", "über", &"a".repeat(49)] {
            assert!(slug_shape(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn image_builds() {
        let build = |file: &str, context: Option<&str>| {
            image_build(&ImageBuild {
                file: file.into(),
                context: context.map(str::to_owned),
            })
        };
        let ok = build(" ./docker/Dockerfile ", Some("./")).ok();
        assert_eq!(
            ok,
            Some(ImageBuild {
                file: "docker/Dockerfile".into(),
                context: None
            })
        );
        assert_eq!(
            build("Dockerfile", Some("app/"))
                .ok()
                .and_then(|b| b.context),
            Some("app".into())
        );
        assert!(build(".devcontainer/devcontainer.json", None).is_ok());
        assert!(build(".devcontainer/devcontainer.json", Some("..")).is_err());
        assert!(build(".devcontainer.json", Some("app")).is_err());
        for bad in [
            "",
            ".",
            "/etc/passwd",
            "../Dockerfile",
            "a/../../b",
            "a//b",
            "a\\b",
        ] {
            assert!(build(bad, None).is_err(), "{bad:?}");
        }
        assert!(build("Dockerfile", Some("../other")).is_err());
    }

    #[test]
    fn images() {
        for ok in [
            "ubuntu",
            "ghcr.io/acme/agent:1",
            "localhost:5000/a@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        ] {
            assert!(image(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "a b",
            "acme/agent:",
            "acme//agent",
            "ghcr.io/acme/agent@latest",
        ] {
            assert!(image(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn branches() {
        for ok in ["main", "fctl/12-add-login", "feature/x.y"] {
            assert!(branch("branch", ok).is_ok(), "{ok}");
        }
        for bad in [
            "", "-x", "a..b", "a b", "x.lock", "a//b", "a/.b", "a@{b", "x/", "a~b",
        ] {
            assert!(branch("branch", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn env_names() {
        assert!(env_name("name", "API_URL").is_ok());
        for bad in [
            "api_url",
            "1X",
            "",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_BASE_URL",
            "CLAUDE_CODE_USE_BEDROCK",
            "PATH",
        ] {
            assert!(env_name("name", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn efforts() {
        assert_eq!(effort("MAX").ok().as_deref(), Some("max"));
        assert!(effort("extreme").is_err());
    }
}
