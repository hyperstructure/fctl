//! Allowlist patterns: an exact email or `*@domain`.

use crate::validate;
use fctl_proto::ApiError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AllowPattern {
    Email(String),
    Domain(String),
}

impl AllowPattern {
    pub fn parse(input: &str) -> Result<Self, ApiError> {
        let s = input.trim().to_lowercase();
        if let Some(domain) = s.strip_prefix("*@").or_else(|| s.strip_prefix('@')) {
            // Validate the domain by validating a sample address on it.
            validate::email(&format!("x@{domain}"))
                .map_err(|_| ApiError::validation("pattern", "Not a valid domain"))?;
            return Ok(Self::Domain(domain.to_owned()));
        }
        validate::email(&s)
            .map(Self::Email)
            .map_err(|_| ApiError::validation("pattern", "Use an email address or *@domain"))
    }

    /// The canonical text form stored in the database.
    pub fn canonical(&self) -> String {
        match self {
            Self::Email(e) => e.clone(),
            Self::Domain(d) => format!("*@{d}"),
        }
    }

    /// Whether a normalized email matches.
    pub fn matches(&self, email: &str) -> bool {
        match self {
            Self::Email(e) => e == email,
            Self::Domain(d) => email.rsplit_once('@').is_some_and(|(_, dom)| dom == d),
        }
    }

    pub fn is_exact(&self) -> bool {
        matches!(self, Self::Email(_))
    }
}

/// The best matching pattern for `email`: exact entries win over domains.
pub fn best_match<'a, T>(
    entries: &'a [T],
    email: &str,
    pattern_of: impl Fn(&T) -> &str,
) -> Option<&'a T> {
    let mut domain_hit = None;
    for entry in entries {
        let Ok(p) = AllowPattern::parse(pattern_of(entry)) else {
            continue;
        };
        if p.matches(email) {
            if p.is_exact() {
                return Some(entry);
            }
            domain_hit = domain_hit.or(Some(entry));
        }
    }
    domain_hit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        assert_eq!(
            AllowPattern::parse(" Ada@X.com").ok(),
            Some(AllowPattern::Email("ada@x.com".into()))
        );
        assert_eq!(
            AllowPattern::parse("*@Corp.io").ok(),
            Some(AllowPattern::Domain("corp.io".into()))
        );
        assert_eq!(
            AllowPattern::parse("@corp.io")
                .map(|p| p.canonical())
                .ok()
                .as_deref(),
            Some("*@corp.io")
        );
        assert!(AllowPattern::parse("*@nodot").is_err());
        assert!(AllowPattern::parse("nobody").is_err());
    }

    #[test]
    fn matching() {
        let d = AllowPattern::Domain("corp.io".into());
        assert!(d.matches("ada@corp.io"));
        assert!(!d.matches("ada@evilcorp.io"));
        assert!(!d.matches("ada@corp.io.evil.com"));
        let entries = vec!["*@corp.io", "ada@corp.io"];
        assert_eq!(
            best_match(&entries, "ada@corp.io", |s| s),
            Some(&"ada@corp.io")
        );
        assert_eq!(
            best_match(&entries, "bob@corp.io", |s| s),
            Some(&"*@corp.io")
        );
        assert_eq!(best_match(&entries, "bob@x.io", |s| s), None);
    }
}
