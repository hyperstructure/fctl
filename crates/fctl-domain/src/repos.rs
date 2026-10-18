//! The repositories a project's agents may use besides its primary one.
//!
//! All of a project's repositories belong to one GitHub account, so one
//! installation of the GitHub App mints every token a session needs: one
//! that can write to the primary repository and the other writable ones, and
//! a read-only one for the rest (every repository of the account, for an
//! owner-wide grant). Owner-wide grants are read-only; agents may write only
//! to repositories that are named.

use crate::validate::Result;
use fctl_proto::ApiError;
use fctl_proto::session::{RepoAccess, RepoGrant, RepoRef};

/// Most repositories a project lists besides its primary one. GitHub mints
/// tokens for at most 500, and every one of them is named in the agent's
/// system prompt.
pub const MAX_REPOS: usize = 100;

/// Parses what people type: `owner/name`, `owner/*` (every repository of
/// `owner`), or a repository's github.com URL. Returns the owner and the
/// name (`None` for an owner-wide grant).
pub fn parse(input: &str) -> Result<(String, Option<String>)> {
    let bad = || {
        ApiError::validation(
            "repo",
            "Use owner/name, or owner/* for every repository of an account",
        )
    };
    let mut s = input.trim();
    for prefix in ["https://github.com/", "http://github.com/", "github.com/"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest;
            break;
        }
    }
    let s = s.trim_end_matches('/');
    if let Some(owner) = s.strip_suffix("/*") {
        // An owner is checked like a name.
        let r = RepoRef::parse(&format!("{owner}/{owner}")).ok_or_else(bad)?;
        return Ok((r.owner, None));
    }
    let r = RepoRef::parse(s).ok_or_else(bad)?;
    Ok((r.owner, Some(r.name)))
}

/// The account a project's repositories belong to.
pub fn owner<'a>(primary: Option<&'a RepoRef>, grants: &'a [RepoGrant]) -> Option<&'a str> {
    primary
        .map(|p| p.owner.as_str())
        .or_else(|| grants.first().map(|g| g.owner.as_str()))
}

fn same_target(a: &RepoGrant, b: &RepoGrant) -> bool {
    a.owner.eq_ignore_ascii_case(&b.owner)
        && match (&a.name, &b.name) {
            (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
            (None, None) => true,
            _ => false,
        }
}

fn other_account(owner: &str) -> ApiError {
    ApiError::validation(
        "repo",
        format!(
            "This project's repositories belong to {owner}. One installation of the GitHub App grants access to all of them, so they can't come from another account."
        ),
    )
}

/// Checks a grant before it is added or changed.
pub fn check(primary: Option<&RepoRef>, grants: &[RepoGrant], new: &RepoGrant) -> Result<()> {
    if new.name.is_none() && new.access == RepoAccess::Write {
        return Err(ApiError::validation(
            "access",
            "Agents can only read every repository of an account. Add the ones they may write to by name.",
        ));
    }
    if let (Some(p), Some(r)) = (primary, new.repo())
        && p.same(&r)
    {
        return Err(ApiError::validation(
            "repo",
            format!(
                "{} is the project's primary repository: agents already write to it",
                p.full_name()
            ),
        ));
    }
    if let Some(owner) = owner(primary, grants)
        && !owner.eq_ignore_ascii_case(&new.owner)
    {
        return Err(other_account(owner));
    }
    let replaces = grants.iter().any(|g| same_target(g, new));
    if !replaces && grants.len() >= MAX_REPOS {
        return Err(ApiError::validation(
            "repo",
            format!(
                "A project can list at most {MAX_REPOS} repositories. Let agents read every repository of the account instead (owner/*)."
            ),
        ));
    }
    Ok(())
}

/// Checks a new primary repository against the project's other ones.
pub fn check_primary(new: &RepoRef, grants: &[RepoGrant]) -> Result<()> {
    match grants.first() {
        Some(g) if !g.owner.eq_ignore_ascii_case(&new.owner) => Err(other_account(&g.owner)),
        _ => Ok(()),
    }
}

/// What a session's GitHub tokens cover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenPlan {
    /// The account whose installation of the GitHub App mints them.
    pub owner: String,
    /// The token that can write: the primary repository first.
    pub write: Vec<String>,
    /// The read-only token.
    pub read: ReadScope,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadScope {
    /// No read-only token.
    None,
    /// For these repositories.
    Repos(Vec<String>),
    /// For every repository the installation can reach (an owner-wide grant).
    All,
}

/// The tokens a session needs, or `None` when it has no repository.
pub fn token_plan(primary: Option<&RepoRef>, grants: &[RepoGrant]) -> Option<TokenPlan> {
    let owner = owner(primary, grants)?.to_owned();
    let mut write: Vec<String> = primary.map(|p| p.name.clone()).into_iter().collect();
    let mut read: Vec<String> = Vec::new();
    let mut all = false;
    let push = |list: &mut Vec<String>, name: &str| {
        if !list.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            list.push(name.to_owned());
        }
    };
    // Grants on another account cannot exist (`check`); never mint for them.
    for g in grants
        .iter()
        .filter(|g| g.owner.eq_ignore_ascii_case(&owner))
    {
        match (&g.name, g.access) {
            (None, _) => all = true,
            (Some(n), RepoAccess::Write) => push(&mut write, n),
            (Some(n), RepoAccess::Read) => push(&mut read, n),
        }
    }
    read.retain(|r| !write.iter().any(|w| w.eq_ignore_ascii_case(r)));
    let read = if all {
        ReadScope::All
    } else if read.is_empty() {
        ReadScope::None
    } else {
        ReadScope::Repos(read)
    };
    Some(TokenPlan { owner, write, read })
}

/// What an agent may do in `repo`: write to the primary repository and the
/// writable grants, read the others it is granted.
pub fn access(
    primary: Option<&RepoRef>,
    grants: &[RepoGrant],
    repo: &RepoRef,
) -> Option<RepoAccess> {
    if primary.is_some_and(|p| p.same(repo)) {
        return Some(RepoAccess::Write);
    }
    grants
        .iter()
        .filter(|g| g.owner.eq_ignore_ascii_case(&repo.owner))
        .filter_map(|g| match &g.name {
            Some(n) if n.eq_ignore_ascii_case(&repo.name) => Some(g.access),
            Some(_) => None,
            None => Some(RepoAccess::Read),
        })
        .max()
}

/// The GitHub repository a git remote points at (`https://github.com/o/n.git`,
/// with or without credentials, `git@github.com:o/n.git`, `ssh://…`).
pub fn parse_remote(url: &str) -> Option<RepoRef> {
    let url = url.trim();
    let path = if let Some(rest) = url.strip_prefix("git@github.com:") {
        rest
    } else {
        let rest = ["https://", "http://", "ssh://"]
            .iter()
            .find_map(|scheme| url.strip_prefix(scheme))?;
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?;
        if !host.eq_ignore_ascii_case("github.com") {
            return None;
        }
        path
    };
    RepoRef::parse(path.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(full: &str) -> RepoRef {
        RepoRef::parse(full).unwrap_or_else(|| RepoRef {
            owner: String::new(),
            name: String::new(),
        })
    }

    fn g(label: &str, access: RepoAccess) -> RepoGrant {
        let (owner, name) = parse(label).unwrap_or_default();
        RepoGrant {
            owner,
            name,
            access,
        }
    }

    #[test]
    fn parses_what_people_type() {
        let ok = |s: &str| parse(s).ok();
        assert_eq!(ok("acme/lib"), Some(("acme".into(), Some("lib".into()))));
        assert_eq!(
            ok(" acme/lib.git "),
            Some(("acme".into(), Some("lib".into())))
        );
        assert_eq!(
            ok("https://github.com/acme/lib/"),
            Some(("acme".into(), Some("lib".into())))
        );
        assert_eq!(ok("acme/*"), Some(("acme".into(), None)));
        assert_eq!(ok("github.com/acme/*"), Some(("acme".into(), None)));
        for bad in [
            "acme",
            "acme/",
            "/lib",
            "acme/lib/tree/main",
            "a b/c",
            "*/*",
            "../x",
        ] {
            assert!(parse(bad).is_err(), "{bad} should not parse");
        }
    }

    #[test]
    fn grants_share_one_account() {
        let primary = r("acme/app");
        let ok = |grants: &[RepoGrant], new: RepoGrant| check(Some(&primary), grants, &new);
        assert!(ok(&[], g("acme/lib", RepoAccess::Write)).is_ok());
        assert!(ok(&[], g("ACME/lib", RepoAccess::Read)).is_ok());
        let other = ok(&[], g("other/lib", RepoAccess::Read));
        assert_eq!(other.map_err(|e| e.field), Err(Some("repo".into())));
        // Without a primary repository, the first grant picks the account.
        let first = [g("acme/lib", RepoAccess::Read)];
        assert!(check(None, &first, &g("acme/*", RepoAccess::Read)).is_ok());
        assert!(check(None, &first, &g("other/x", RepoAccess::Read)).is_err());
        assert!(check_primary(&r("acme/new"), &first).is_ok());
        assert!(check_primary(&r("other/new"), &first).is_err());
        assert!(check_primary(&r("other/new"), &[]).is_ok());
    }

    #[test]
    fn owner_wide_grants_are_read_only() {
        let err = check(None, &[], &g("acme/*", RepoAccess::Write));
        assert_eq!(err.map_err(|e| e.field), Err(Some("access".into())));
        assert!(check(None, &[], &g("acme/*", RepoAccess::Read)).is_ok());
    }

    #[test]
    fn the_primary_repository_is_not_listed_again() {
        let primary = r("acme/app");
        assert!(check(Some(&primary), &[], &g("Acme/App", RepoAccess::Read)).is_err());
    }

    #[test]
    fn lists_are_bounded_but_changes_are_not() {
        let grants: Vec<RepoGrant> = (0..MAX_REPOS)
            .map(|i| g(&format!("acme/r{i}"), RepoAccess::Read))
            .collect();
        assert!(check(None, &grants, &g("acme/one-more", RepoAccess::Read)).is_err());
        assert!(check(None, &grants, &g("acme/r7", RepoAccess::Write)).is_ok());
    }

    #[test]
    fn plans_one_token_per_access_level() {
        let primary = r("acme/app");
        assert_eq!(token_plan(None, &[]), None);
        assert_eq!(
            token_plan(Some(&primary), &[]),
            Some(TokenPlan {
                owner: "acme".into(),
                write: vec!["app".into()],
                read: ReadScope::None,
            })
        );
        let grants = [
            g("acme/lib", RepoAccess::Write),
            g("acme/docs", RepoAccess::Read),
            g("acme/LIB", RepoAccess::Read),
            g("other/x", RepoAccess::Write),
        ];
        assert_eq!(
            token_plan(Some(&primary), &grants),
            Some(TokenPlan {
                owner: "acme".into(),
                write: vec!["app".into(), "lib".into()],
                read: ReadScope::Repos(vec!["docs".into()]),
            })
        );
        let wide = [
            g("acme/lib", RepoAccess::Write),
            g("acme/*", RepoAccess::Read),
        ];
        assert_eq!(
            token_plan(None, &wide),
            Some(TokenPlan {
                owner: "acme".into(),
                write: vec!["lib".into()],
                read: ReadScope::All,
            })
        );
    }

    #[test]
    fn access_follows_the_grants() {
        let primary = r("acme/app");
        let grants = [
            g("acme/lib", RepoAccess::Write),
            g("acme/docs", RepoAccess::Read),
        ];
        let a = |full: &str| access(Some(&primary), &grants, &r(full));
        assert_eq!(a("acme/app"), Some(RepoAccess::Write));
        assert_eq!(a("Acme/Lib"), Some(RepoAccess::Write));
        assert_eq!(a("acme/docs"), Some(RepoAccess::Read));
        assert_eq!(a("acme/other"), None);
        let wide = [
            g("acme/*", RepoAccess::Read),
            g("acme/lib", RepoAccess::Write),
        ];
        assert_eq!(
            access(None, &wide, &r("acme/other")),
            Some(RepoAccess::Read)
        );
        assert_eq!(access(None, &wide, &r("acme/lib")), Some(RepoAccess::Write));
        assert_eq!(access(None, &wide, &r("other/lib")), None);
    }

    #[test]
    fn reads_remotes() {
        let p = |u: &str| parse_remote(u).map(|r| r.full_name());
        assert_eq!(
            p("https://github.com/acme/lib.git"),
            Some("acme/lib".into())
        );
        assert_eq!(p("https://github.com/acme/lib"), Some("acme/lib".into()));
        assert_eq!(
            p("https://x-access-token:ghs_x@github.com/acme/lib.git"),
            Some("acme/lib".into())
        );
        assert_eq!(p("git@github.com:acme/lib.git"), Some("acme/lib".into()));
        assert_eq!(
            p("ssh://git@github.com/acme/lib.git"),
            Some("acme/lib".into())
        );
        assert_eq!(p("https://gitlab.com/acme/lib.git"), None);
        assert_eq!(p("/srv/git/lib"), None);
    }
}
