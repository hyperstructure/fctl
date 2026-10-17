//! Input validation. Every function returns the cleaned value or an
//! [`ApiError`] naming the field, so handlers can return it as is.

use fctl_proto::ApiError;

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

/// A display name: 1–80 characters, no control characters.
pub fn name(field: &str, input: &str) -> Result<String> {
    let s = collapse_spaces(input);
    if s.is_empty() {
        return Err(ApiError::validation(field, "Enter a name"));
    }
    if s.chars().count() > 80 {
        return Err(ApiError::validation(field, "Use at most 80 characters"));
    }
    Ok(s)
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

fn collapse_spaces(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Slugs that would collide with routes.
const RESERVED_SLUGS: &[&str] = &[
    "api", "admin", "new", "settings", "login", "logout", "signup", "setup", "device", "invite",
    "s", "p", "projects", "sessions", "runners", "assets", "static", "app", "help", "docs",
];

/// A URL slug: 2–48 of `a-z 0-9 -`, no leading, trailing or double dashes.
pub fn slug(input: &str) -> Result<String> {
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
    if RESERVED_SLUGS.contains(&s.as_str()) {
        return Err(bad("This name is reserved"));
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
    {
        return Err(ApiError::validation("image", "Not a valid image reference"));
    }
    Ok(s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

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
