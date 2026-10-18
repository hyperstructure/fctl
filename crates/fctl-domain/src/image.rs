//! Sandbox images: references as registries read them, the images and
//! builds a host allows, and what a session's project asks for.

use fctl_proto::session::{ProjectSettings, RepoRef};

/// The name the default image had before it was published, which hosts and
/// projects from then may still have.
pub const OLD_DEFAULT_IMAGE: &str = "fctl-agent:latest";

/// Whether an image setting, a host's or a project's, means the host's
/// default: empty, or the name the default had before it was published.
pub fn means_default(image: &str) -> bool {
    let image = image.trim();
    image.is_empty() || image == OLD_DEFAULT_IMAGE
}

/// An image reference as Docker reads it: `ubuntu` is
/// `docker.io/library/ubuntu`, at the tag `latest`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRef {
    /// The registry and the repository in it, lowercase:
    /// `ghcr.io/acme/agent`.
    pub name: String,
    /// Absent when the reference names none: `latest`, unless it names a
    /// digest.
    pub tag: Option<String>,
    /// `sha256:…`.
    pub digest: Option<String>,
}

impl ImageRef {
    pub fn parse(reference: &str) -> Option<Self> {
        let s = reference.trim();
        if s.is_empty() || s.len() > 255 || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return None;
        }
        let (rest, digest) = match s.split_once('@') {
            Some((rest, digest)) => (rest, Some(digest)),
            None => (s, None),
        };
        if let Some(d) = digest
            && !valid_digest(d)
        {
            return None;
        }
        // A tag follows the last colon after the last slash; one before it
        // separates a registry's port.
        let last = rest.rfind('/').map_or(0, |i| i + 1);
        let (path, tag) = match rest[last..].rfind(':') {
            Some(i) => (&rest[..last + i], Some(&rest[last + i + 1..])),
            None => (rest, None),
        };
        if let Some(t) = tag
            && !valid_tag(t)
        {
            return None;
        }
        let name = full_name(path)?;
        Some(Self {
            name,
            tag: tag.map(str::to_owned),
            digest: digest.map(str::to_ascii_lowercase),
        })
    }

    /// The tag a pull without a digest gets.
    fn effective_tag(&self) -> Option<&str> {
        match (&self.tag, &self.digest) {
            (Some(t), _) => Some(t),
            (None, None) => Some("latest"),
            (None, Some(_)) => None,
        }
    }

    /// Whether both are the same image: the same repository, tag and digest.
    pub fn same(&self, other: &Self) -> bool {
        self.name == other.name
            && self.effective_tag() == other.effective_tag()
            && self.digest == other.digest
    }
}

/// `sha256:` and 64 hex digits (or another algorithm and its digits).
fn valid_digest(d: &str) -> bool {
    let Some((algorithm, hex)) = d.split_once(':') else {
        return false;
    };
    !algorithm.is_empty()
        && algorithm
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+._-".contains(&b))
        && hex.len() >= 32
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

fn valid_tag(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= 128
        && !t.starts_with(['.', '-'])
        && t.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

/// `registry/path` for a repository as written: Docker Hub's when the first
/// part names no registry (no dot, colon or `localhost`), and its official
/// images under `library/`.
fn full_name(path: &str) -> Option<String> {
    let (registry, repo) = match path.split_once('/') {
        Some((first, rest))
            if first.contains('.') || first.contains(':') || first == "localhost" =>
        {
            (first.to_ascii_lowercase(), rest)
        }
        _ => ("docker.io".to_owned(), path),
    };
    let registry = match registry.as_str() {
        "index.docker.io" | "registry-1.docker.io" => "docker.io".to_owned(),
        _ => registry,
    };
    let repo = repo.to_ascii_lowercase();
    let component_ok = |c: &str| {
        !c.is_empty()
            && c.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
            && c.starts_with(|ch: char| ch.is_ascii_alphanumeric())
            && c.ends_with(|ch: char| ch.is_ascii_alphanumeric())
    };
    if !repo.split('/').all(component_ok) {
        return None;
    }
    let repo = if registry == "docker.io" && !repo.contains('/') {
        format!("library/{repo}")
    } else {
        repo
    };
    Some(format!("{registry}/{repo}"))
}

/// Why an entry of `allowed_images` matches nothing, or `None` when it is
/// one: `*`, a prefix that ends in `/*` (`ghcr.io/acme/*`), or a reference,
/// with or without a tag, pinned to a digest or not.
pub fn check_image_pattern(pattern: &str) -> Option<String> {
    let p = pattern.trim();
    if p == "*" {
        return None;
    }
    if let Some(prefix) = p.strip_suffix("/*") {
        return full_prefix(prefix)
            .is_none()
            .then(|| format!("`{p}` is not a registry or a path in one"));
    }
    if p.ends_with('*') {
        return Some(format!("`{p}`: a prefix ends in /*, like ghcr.io/acme/*"));
    }
    ImageRef::parse(p)
        .is_none()
        .then(|| format!("`{p}` is not an image reference"))
}

/// `ghcr.io/acme` from `ghcr.io/acme`; `docker.io/acme` from `acme`.
fn full_prefix(prefix: &str) -> Option<String> {
    let prefix = prefix.trim().trim_end_matches('/');
    if prefix.is_empty() {
        return None;
    }
    // A bare registry: `ghcr.io/*`, `localhost:5000/*`.
    if !prefix.contains('/')
        && (prefix.contains('.') || prefix.contains(':') || prefix == "localhost")
    {
        return Some(prefix.to_ascii_lowercase());
    }
    let name = full_name(prefix)?;
    // `acme/*` is Docker Hub's acme, not `library/acme`.
    Some(match name.strip_prefix("docker.io/library/") {
        Some(rest) if !prefix.contains('/') => format!("docker.io/{rest}"),
        _ => name,
    })
}

/// Whether `pattern`, an entry of a host's `allowed_images`, allows `image`.
///
/// - `*`: any image.
/// - `ghcr.io/acme/*`: any image in that registry path.
/// - `ghcr.io/acme/agent`: any tag or digest of that repository.
/// - `ghcr.io/acme/agent:1`: that tag, pinned to a digest or not.
/// - `ghcr.io/acme/agent@sha256:…`: only that digest, which the project's
///   reference must name too: a tag can move.
pub fn pattern_allows(pattern: &str, image: &ImageRef) -> bool {
    let p = pattern.trim();
    if p == "*" {
        return true;
    }
    if let Some(prefix) = p.strip_suffix("/*") {
        return full_prefix(prefix).is_some_and(|prefix| {
            image
                .name
                .strip_prefix(&prefix)
                .is_some_and(|rest| rest.starts_with('/'))
        });
    }
    let Some(p) = ImageRef::parse(p) else {
        return false;
    };
    if p.name != image.name {
        return false;
    }
    match (&p.digest, &p.tag) {
        (Some(digest), _) => image.digest.as_ref() == Some(digest),
        (None, Some(tag)) => image.effective_tag() == Some(tag.as_str()),
        (None, None) => true,
    }
}

/// Why an entry of `allowed_builds` matches no repository, or `None` when it
/// is one: `owner/name`, `owner/*`, or `*`.
pub fn check_build_pattern(pattern: &str) -> Option<String> {
    let p = pattern.trim();
    if p == "*" {
        return None;
    }
    crate::repos::parse(p)
        .err()
        .map(|_| format!("`{p}` is not owner/name or owner/*"))
}

/// Whether `pattern`, an entry of a host's `allowed_builds`, allows builds
/// from `repo`'s files. GitHub ignores case.
pub fn build_pattern_allows(pattern: &str, repo: &RepoRef) -> bool {
    let p = pattern.trim();
    if p == "*" {
        return true;
    }
    match crate::repos::parse(p) {
        Ok((owner, None)) => owner.eq_ignore_ascii_case(&repo.owner),
        Ok((owner, Some(name))) => {
            owner.eq_ignore_ascii_case(&repo.owner) && name.eq_ignore_ascii_case(&repo.name)
        }
        Err(_) => false,
    }
}

/// What a session's sandbox starts from, as its project's settings say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ImageWant {
    /// The host's own image.
    #[default]
    Default,
    /// An image from a registry.
    Image(String),
    /// An image built from a file in this repository.
    Build(RepoRef),
}

impl ImageWant {
    /// What sessions of a project with these settings, and this primary
    /// repository, start from. A build without a repository has nothing to
    /// build from, and the control plane refuses one.
    pub fn of(settings: &ProjectSettings, repo: Option<&RepoRef>) -> Self {
        if settings.image_build.is_some()
            && let Some(repo) = repo
        {
            return Self::Build(repo.clone());
        }
        match settings.image.as_deref() {
            Some(image) if !means_default(image) => Self::Image(image.trim().to_owned()),
            _ => Self::Default,
        }
    }
}

/// The images a host runs and the repositories it builds images from, as
/// its hello says.
#[derive(Clone, Copy, Debug, Default)]
pub struct HostImages<'a> {
    /// The image it gives sessions whose project sets none. It runs that
    /// one whatever `allowed` says.
    pub own: Option<&'a str>,
    /// `allowed_images`. `None`, as from hosts from before it: any image.
    pub allowed: Option<&'a [String]>,
    /// `allowed_builds`, sent only by hosts that can build. `None`, as from
    /// hosts from before it: none.
    pub builds: Option<&'a [String]>,
}

impl HostImages<'_> {
    /// Whether the host runs sessions whose project chose `image`.
    pub fn runs(&self, image: &str) -> bool {
        if means_default(image) {
            return true;
        }
        let Some(allowed) = self.allowed else {
            return true;
        };
        let Some(image) = ImageRef::parse(image) else {
            return false;
        };
        let own = self
            .own
            .and_then(ImageRef::parse)
            .is_some_and(|own| own.same(&image));
        own || allowed.iter().any(|p| pattern_allows(p, &image))
    }

    /// Whether the host builds images from `repo`'s files.
    pub fn builds_from(&self, repo: &RepoRef) -> bool {
        self.builds
            .is_some_and(|b| b.iter().any(|p| build_pattern_allows(p, repo)))
    }

    /// Whether the host can give a session what it wants to start from.
    pub fn takes(&self, want: &ImageWant) -> bool {
        match want {
            ImageWant::Default => true,
            ImageWant::Image(image) => self.runs(image),
            ImageWant::Build(repo) => self.builds_from(repo),
        }
    }
}

/// Which images a host runs for projects that choose their own, in words:
/// `any image`, `only its own image`, `its own image and ghcr.io/acme/*`.
pub fn describe_allowed(allowed: Option<&[String]>) -> String {
    match allowed {
        None => "any image".into(),
        Some(list) if list.iter().any(|p| p.trim() == "*") => "any image".into(),
        Some([]) => "only its own image".into(),
        Some(list) => format!("its own image and {}", list.join(", ")),
    }
}

/// Which repositories a host builds images from, in words: `none`,
/// `acme/*, hyperstructure/fctl`, `any repository`.
pub fn describe_builds(builds: Option<&[String]>) -> String {
    match builds {
        None | Some([]) => "none".into(),
        Some(list) if list.iter().any(|p| p.trim() == "*") => "any repository".into(),
        Some(list) => list.join(", "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::session::ImageBuild;

    fn r(s: &str) -> ImageRef {
        ImageRef::parse(s).unwrap_or_else(|| ImageRef {
            name: format!("unparsable {s}"),
            tag: None,
            digest: None,
        })
    }

    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn reads_references_as_docker_does() {
        assert_eq!(
            ImageRef::parse("ubuntu"),
            Some(ImageRef {
                name: "docker.io/library/ubuntu".into(),
                tag: None,
                digest: None
            })
        );
        assert_eq!(r("acme/tools:2").name, "docker.io/acme/tools");
        assert_eq!(r("acme/tools:2").tag.as_deref(), Some("2"));
        assert_eq!(
            r("index.docker.io/library/ubuntu").name,
            "docker.io/library/ubuntu"
        );
        let pinned = r(&format!("GHCR.io/Acme/Agent:1@{DIGEST}"));
        assert_eq!(pinned.name, "ghcr.io/acme/agent");
        assert_eq!(pinned.tag.as_deref(), Some("1"));
        assert_eq!(pinned.digest.as_deref(), Some(DIGEST));
        // A port is not a tag.
        let local = r("localhost:5000/agent");
        assert_eq!(local.name, "localhost:5000/agent");
        assert_eq!(local.tag, None);
        assert_eq!(r("registry.acme.dev:5000/x/y:3").tag.as_deref(), Some("3"));
        for bad in [
            "",
            "two words",
            "acme/agent:",
            "acme/agent@sha256:short",
            "acme//agent",
            "ghcr.io/acme/-agent",
            "acme/agent:-1",
        ] {
            assert_eq!(ImageRef::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn patterns_allow_registries_repositories_tags_and_digests() {
        let agent = r("ghcr.io/acme/agent:1");
        assert!(pattern_allows("*", &agent));
        assert!(pattern_allows("ghcr.io/acme/*", &agent));
        assert!(pattern_allows("ghcr.io/*", &agent));
        assert!(pattern_allows("GHCR.IO/acme/*", &agent));
        assert!(!pattern_allows("ghcr.io/acm/*", &agent));
        assert!(!pattern_allows("ghcr.io/acme/agent/*", &agent));
        assert!(pattern_allows("ghcr.io/acme/agent", &agent));
        assert!(pattern_allows("ghcr.io/acme/agent:1", &agent));
        assert!(!pattern_allows("ghcr.io/acme/agent:2", &agent));
        // A tag allows itself pinned to any digest, a digest only itself.
        let pinned = r(&format!("ghcr.io/acme/agent:1@{DIGEST}"));
        assert!(pattern_allows("ghcr.io/acme/agent:1", &pinned));
        assert!(pattern_allows(
            &format!("ghcr.io/acme/agent@{DIGEST}"),
            &pinned
        ));
        assert!(!pattern_allows(
            &format!("ghcr.io/acme/agent@{DIGEST}"),
            &agent
        ));
        let by_digest = r(&format!("ghcr.io/acme/agent@{DIGEST}"));
        assert!(!pattern_allows("ghcr.io/acme/agent:latest", &by_digest));
        assert!(pattern_allows("ghcr.io/acme/agent", &by_digest));
        // No tag is `latest`.
        assert!(pattern_allows(
            "ghcr.io/acme/agent:latest",
            &r("ghcr.io/acme/agent")
        ));
        // Docker Hub names.
        assert!(pattern_allows(
            "ubuntu",
            &r("docker.io/library/ubuntu:24.04")
        ));
        assert!(pattern_allows("acme/*", &r("acme/tools")));
        assert!(pattern_allows("docker.io/*", &r("ubuntu")));
        assert!(!pattern_allows("acme/*", &r("ghcr.io/acme/tools")));
        // Not patterns.
        assert!(!pattern_allows("ghcr.io/acme*", &agent));
        assert!(!pattern_allows("", &agent));
    }

    #[test]
    fn checks_patterns() {
        for good in [
            "*",
            "ghcr.io/*",
            "ghcr.io/acme/*",
            "acme/*",
            "ubuntu",
            "ghcr.io/a/b:1",
        ] {
            assert_eq!(check_image_pattern(good), None, "{good}");
        }
        assert!(check_image_pattern("ghcr.io/acme*").is_some_and(|e| e.contains("/*")));
        assert!(check_image_pattern("two words").is_some());
        for good in ["*", "acme/*", "acme/app", "https://github.com/acme/app"] {
            assert_eq!(check_build_pattern(good), None, "{good}");
        }
        assert!(check_build_pattern("acme").is_some());
    }

    #[test]
    fn builds_are_allowed_by_repository() {
        let app = RepoRef {
            owner: "Acme".into(),
            name: "App".into(),
        };
        assert!(build_pattern_allows("*", &app));
        assert!(build_pattern_allows("acme/*", &app));
        assert!(build_pattern_allows("acme/app", &app));
        assert!(build_pattern_allows("github.com/acme/app", &app));
        assert!(!build_pattern_allows("acme/web", &app));
        assert!(!build_pattern_allows("other/*", &app));
    }

    #[test]
    fn hosts_run_their_own_image_and_the_ones_they_allow() {
        let list = vec!["ghcr.io/acme/*".to_owned()];
        let host = HostImages {
            own: Some("ghcr.io/hyperstructure/sandbox:0.5.0"),
            allowed: Some(&list),
            builds: None,
        };
        assert!(host.runs(""));
        assert!(host.runs("fctl-agent:latest"));
        assert!(host.runs("ghcr.io/acme/agent:1"));
        assert!(host.runs("ghcr.io/hyperstructure/sandbox:0.5.0"));
        assert!(!host.runs("ghcr.io/hyperstructure/sandbox:0.4.0"));
        assert!(!host.runs("docker.io/evil/miner"));
        assert!(!host.runs("not an image"));
        // Without a list, any image: hosts from before `allowed_images`.
        let any = HostImages {
            own: None,
            allowed: None,
            builds: None,
        };
        assert!(any.runs("docker.io/evil/miner"));
        // An empty list: only its own.
        let none: Vec<String> = vec![];
        let own_only = HostImages {
            allowed: Some(&none),
            ..host
        };
        assert!(own_only.runs("ghcr.io/hyperstructure/sandbox:0.5.0"));
        assert!(!own_only.runs("ghcr.io/acme/agent:1"));
    }

    #[test]
    fn says_what_hosts_allow() {
        let list = |l: &[&str]| l.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(describe_allowed(None), "any image");
        assert_eq!(describe_allowed(Some(&[])), "only its own image");
        assert_eq!(
            describe_allowed(Some(&list(&["ghcr.io/acme/*", "ubuntu"]))),
            "its own image and ghcr.io/acme/*, ubuntu"
        );
        assert_eq!(describe_allowed(Some(&list(&["*"]))), "any image");
        assert_eq!(describe_builds(None), "none");
        assert_eq!(describe_builds(Some(&list(&["acme/*"]))), "acme/*");
        assert_eq!(describe_builds(Some(&list(&["*"]))), "any repository");
    }

    #[test]
    fn sessions_want_their_projects_image_or_build() {
        let repo = RepoRef {
            owner: "acme".into(),
            name: "app".into(),
        };
        let mut s = ProjectSettings::default();
        assert_eq!(ImageWant::of(&s, Some(&repo)), ImageWant::Default);
        s.image = Some("fctl-agent:latest".into());
        assert_eq!(ImageWant::of(&s, Some(&repo)), ImageWant::Default);
        s.image = Some(" ghcr.io/acme/agent:1 ".into());
        assert_eq!(
            ImageWant::of(&s, Some(&repo)),
            ImageWant::Image("ghcr.io/acme/agent:1".into())
        );
        s.image = None;
        s.image_build = Some(ImageBuild {
            file: "Dockerfile".into(),
            context: None,
        });
        assert_eq!(
            ImageWant::of(&s, Some(&repo)),
            ImageWant::Build(repo.clone())
        );
        assert_eq!(ImageWant::of(&s, None), ImageWant::Default);

        let builders = vec!["acme/*".to_owned()];
        let host = HostImages {
            own: None,
            allowed: None,
            builds: Some(&builders),
        };
        assert!(host.takes(&ImageWant::Build(repo.clone())));
        assert!(!HostImages::default().takes(&ImageWant::Build(repo)));
    }
}
