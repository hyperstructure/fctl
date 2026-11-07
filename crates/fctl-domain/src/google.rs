//! Google sign-in: which addresses Google vouches for.
//!
//! A Google account may show any address its person verified once, when
//! they made the account, Gmail or not. Google is authoritative only for
//! the addresses it hosts: Gmail's, and those of Google Workspace (and
//! Cloud Identity) accounts, whose ID tokens name their domain (`hd`). Any
//! other address may have changed hands since (a former employee's, a
//! recycled role mailbox), while the Google account still shows it as
//! verified
//! ([Google's guidance](https://developers.google.com/identity/openid-connect/openid-connect#obtaininguserprofileinformation)).
//! So a Google sign-in makes an account for an address only where Google
//! vouches for it; anyone else signs up with the invitation sent to it,
//! which only its mailbox gets.

/// The domains of Gmail addresses, which Google hosts.
const GMAIL: [&str; 2] = ["gmail.com", "googlemail.com"];

/// Whether Google vouches that the person signing in holds `email` now: it
/// verified it, and hosts it (Gmail), or it is a Google Workspace
/// account's (`hd`, the hosted domain its ID token names).
pub fn vouches_for(email: &str, email_verified: bool, hd: Option<&str>) -> bool {
    if !email_verified {
        return false;
    }
    let Some((_, domain)) = email.rsplit_once('@') else {
        return false;
    };
    let domain = domain.trim().to_ascii_lowercase();
    GMAIL.contains(&domain.as_str()) || hd.is_some_and(|hd| !hd.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail_addresses_are_googles() {
        assert!(vouches_for("ada@gmail.com", true, None));
        assert!(vouches_for("ada@GMail.com", true, None));
        assert!(vouches_for("ada@googlemail.com", true, None));
        // Unverified, never.
        assert!(!vouches_for("ada@gmail.com", false, None));
    }

    #[test]
    fn workspace_accounts_name_their_domain() {
        assert!(vouches_for("ada@acme.io", true, Some("acme.io")));
        // A Workspace account's address may be in another of its
        // organization's domains.
        assert!(vouches_for("ada@acme.dev", true, Some("acme.io")));
        assert!(!vouches_for("ada@acme.io", false, Some("acme.io")));
        assert!(!vouches_for("ada@acme.io", true, Some(" ")));
    }

    #[test]
    fn other_addresses_google_only_verified_once() {
        // A Google account made with an address of a domain Google doesn't
        // host: whoever made it may have lost the mailbox since.
        assert!(!vouches_for("ada@acme.io", true, None));
        assert!(!vouches_for("ada@gmail.com.acme.io", true, None));
        assert!(!vouches_for("not an address", true, None));
    }
}
