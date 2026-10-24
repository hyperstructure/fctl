//! What the app tells search engines: `/robots.txt` and `/sitemap.xml`.
//!
//! An instance is closed to crawlers unless its deployment opens it
//! (`search_engines` in deploy/instances, which sets `FCTL_SEARCH_ENGINES`;
//! app.fctl.sh's does). An open instance lets them read its public pages
//! (the sign-in page, the early access page where it has one, and
//! `/llms.txt`) and lists those in its sitemap; everything else stays
//! disallowed, though it needs signing in anyway.

/// The public pages an open instance lists, in order.
pub fn public_pages(early_access: bool) -> Vec<&'static str> {
    let mut pages = vec!["/login"];
    if early_access {
        pages.push("/early-access");
    }
    pages.push("/llms.txt");
    pages
}

/// `/robots.txt`. `public_url` has no trailing slash.
pub fn robots_txt(public_url: &str, open: bool, early_access: bool) -> String {
    if !open {
        return "User-agent: *\nDisallow: /\n".to_owned();
    }
    let mut out = String::from("User-agent: *\nAllow: /$\n");
    for page in public_pages(early_access) {
        out.push_str(&format!("Allow: {page}\n"));
    }
    out.push_str(&format!(
        "Disallow: /\n\nSitemap: {public_url}/sitemap.xml\n"
    ));
    out
}

/// `/sitemap.xml` of an open instance. `public_url` has no trailing slash.
pub fn sitemap_xml(public_url: &str, early_access: bool) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for page in public_pages(early_access) {
        out.push_str(&format!(
            "  <url><loc>{}</loc></url>\n",
            xml_escape(&format!("{public_url}{page}"))
        ));
    }
    out.push_str("</urlset>\n");
    out
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_instance_disallows_everything() {
        assert_eq!(
            robots_txt("https://fctl.example.com", false, true),
            "User-agent: *\nDisallow: /\n"
        );
    }

    #[test]
    fn an_open_instance_allows_its_public_pages_and_names_its_sitemap() {
        assert_eq!(
            robots_txt("https://app.fctl.sh", true, true),
            "User-agent: *\n\
             Allow: /$\n\
             Allow: /login\n\
             Allow: /early-access\n\
             Allow: /llms.txt\n\
             Disallow: /\n\
             \n\
             Sitemap: https://app.fctl.sh/sitemap.xml\n"
        );
    }

    #[test]
    fn the_sitemap_lists_the_public_pages() {
        let xml = sitemap_xml("https://app.fctl.sh", true);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset "));
        assert!(xml.contains("<loc>https://app.fctl.sh/login</loc>"));
        assert!(xml.contains("<loc>https://app.fctl.sh/early-access</loc>"));
        assert!(xml.contains("<loc>https://app.fctl.sh/llms.txt</loc>"));
        assert!(xml.ends_with("</urlset>\n"));
    }

    #[test]
    fn without_early_access_there_is_no_early_access_page() {
        assert!(!sitemap_xml("https://fctl.example.com", false).contains("early-access"));
        assert!(!robots_txt("https://fctl.example.com", true, false).contains("early-access"));
    }

    #[test]
    fn urls_are_escaped() {
        assert!(sitemap_xml("https://a.example/x&y", false).contains("x&amp;y/login"));
    }
}
