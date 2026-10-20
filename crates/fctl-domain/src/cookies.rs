//! The cookies the web app signs in with: the session, a Google sign-in on
//! its way and a sign-in that waits for its second factor; and a GitHub
//! sign-in on its way, which connects a GitHub account to a workspace.
//!
//! Their names carry the `__Host-` prefix, which binds a cookie to the app's
//! own host and path. Browsers take such a cookie only with `Secure`, from a
//! secure origin, so an instance served over plain http at an address other
//! than localhost (a self-hosted one on a LAN, say) can't set it: there the
//! same cookies go without the prefix and without `Secure`.

use std::net::{Ipv4Addr, Ipv6Addr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cookie {
    /// The web session (`fctl_ws_…`).
    Session,
    /// A Google sign-in on its way: its signed state.
    GoogleSignIn,
    /// A sign-in that waits for its second factor (`fctl_2fa_…`).
    SecondFactor,
    /// A GitHub sign-in on its way, to connect an account to a workspace:
    /// its signed state.
    GithubConnect,
}

impl Cookie {
    /// Its name, where cookies can be `Secure` or not.
    pub fn name(self, secure: bool) -> &'static str {
        match (self, secure) {
            (Self::Session, true) => "__Host-fctl",
            (Self::Session, false) => "fctl",
            (Self::GoogleSignIn, true) => "__Host-fctl-oauth",
            (Self::GoogleSignIn, false) => "fctl-oauth",
            (Self::SecondFactor, true) => "__Host-fctl-2fa",
            (Self::SecondFactor, false) => "fctl-2fa",
            (Self::GithubConnect, true) => "__Host-fctl-github",
            (Self::GithubConnect, false) => "fctl-github",
        }
    }
}

/// Whether browsers keep `Secure` cookies from the app at `public_url`
/// (`https://app.example.com`): over https, and over http on localhost and
/// loopback addresses, which they count as secure.
pub fn secure(public_url: &str) -> bool {
    let url = public_url.trim().to_ascii_lowercase();
    if url.starts_with("https://") {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if let Some(v6) = authority.strip_prefix('[') {
        let host = v6.split(']').next().unwrap_or_default();
        return host.parse::<Ipv6Addr>().is_ok_and(|ip| ip.is_loopback());
    }
    let host = authority.split(':').next().unwrap_or_default();
    host == "localhost"
        || host.ends_with(".localhost")
        || host.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_and_localhost_are_secure() {
        for url in [
            "https://app.fctl.sh",
            "https://fctl.acme.dev/",
            "HTTPS://Fctl.Acme.Dev",
            "http://localhost:8787",
            "http://localhost",
            "http://fctl.localhost:8787",
            "http://127.0.0.1:8787",
            "http://127.1.2.3",
            "http://[::1]:8787",
        ] {
            assert!(secure(url), "{url}");
        }
    }

    #[test]
    fn plain_http_elsewhere_is_not() {
        for url in [
            "http://192.168.1.20:8787",
            "http://fctl.lan",
            "http://localhost.example.com",
            "http://127.0.0.1.example.com",
            "http://127.a.b.c",
            "http://[fd00::1]:8787",
            "ftp://localhost",
            "",
        ] {
            assert!(!secure(url), "{url}");
        }
    }

    #[test]
    fn names_drop_the_host_prefix_without_secure() {
        for cookie in [
            Cookie::Session,
            Cookie::GoogleSignIn,
            Cookie::SecondFactor,
            Cookie::GithubConnect,
        ] {
            let secure = cookie.name(true);
            let plain = cookie.name(false);
            assert_eq!(secure.strip_prefix("__Host-"), Some(plain));
        }
        assert_eq!(Cookie::Session.name(true), "__Host-fctl");
    }
}
