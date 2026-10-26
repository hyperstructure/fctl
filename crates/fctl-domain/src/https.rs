//! Plain HTTP on an instance that takes only HTTPS (`https_only` in
//! deploy/instances, which sets `FCTL_HTTPS_ONLY`; app.fctl.sh's does): every
//! request, for a page, an asset or the API, goes to the same address over
//! HTTPS. Other instances answer plain HTTP too, as a server tried out on a
//! network its owner trusts does.

/// Where a request for `url` goes instead, and with which status, when it
/// came over plain HTTP: the same address over HTTPS, on its default port,
/// permanently. 301 for reads, which browsers and search engines know best,
/// and 308 for the rest, which keeps their method and body.
pub fn redirect(url: &str, method: &str) -> Option<(String, u16)> {
    let rest = url.strip_prefix("http://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    let host = match authority.rsplit_once(':') {
        Some((host, port))
            if !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit())
                && (!host.contains(':') || host.ends_with(']')) =>
        {
            host
        }
        _ => authority,
    };
    let read = method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD");
    Some((
        format!("https://{host}{path}"),
        if read { 301 } else { 308 },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_http_goes_to_the_same_address_over_https() {
        assert_eq!(
            redirect("http://app.fctl.sh/s/ses_1?tab=files#end", "GET"),
            Some(("https://app.fctl.sh/s/ses_1?tab=files#end".to_owned(), 301))
        );
        assert_eq!(
            redirect("http://app.fctl.sh/", "HEAD"),
            Some(("https://app.fctl.sh/".to_owned(), 301))
        );
    }

    #[test]
    fn https_stays() {
        assert_eq!(redirect("https://app.fctl.sh/login", "GET"), None);
    }

    #[test]
    fn writes_keep_their_method_and_body() {
        assert_eq!(
            redirect("http://app.fctl.sh/api/v1/sessions", "POST"),
            Some(("https://app.fctl.sh/api/v1/sessions".to_owned(), 308))
        );
        assert_eq!(
            redirect("http://app.fctl.sh/api/v1/sessions/x", "delete").map(|r| r.1),
            Some(308)
        );
    }

    #[test]
    fn a_port_gives_way_to_the_default_https_port() {
        assert_eq!(
            redirect("http://app.fctl.sh:8080/login", "GET").map(|r| r.0),
            Some("https://app.fctl.sh/login".to_owned())
        );
        assert_eq!(
            redirect("http://[2001:db8::1]:80/x", "GET").map(|r| r.0),
            Some("https://[2001:db8::1]/x".to_owned())
        );
        assert_eq!(
            redirect("http://[2001:db8::1]/x", "GET").map(|r| r.0),
            Some("https://[2001:db8::1]/x".to_owned())
        );
        assert_eq!(
            redirect("http://app.fctl.sh?x=1", "GET").map(|r| r.0),
            Some("https://app.fctl.sh?x=1".to_owned())
        );
    }
}
