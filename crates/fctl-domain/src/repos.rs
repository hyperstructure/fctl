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
use fctl_proto::session::{GithubAccess, RepoAccess, RepoGrant, RepoRef};
use serde_json::{Value, json};

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

impl TokenPlan {
    /// The plan of a session whose agent only reads on GitHub: one
    /// read-only token for every repository, those it could write to
    /// besides, so that `gh` reads them all.
    pub fn reading_only(self) -> Self {
        let read = match self.read {
            ReadScope::All => ReadScope::All,
            ReadScope::None if self.write.is_empty() => ReadScope::None,
            ReadScope::None => ReadScope::Repos(self.write),
            ReadScope::Repos(names) => {
                let mut all = self.write;
                all.extend(names);
                ReadScope::Repos(all)
            }
        };
        Self {
            owner: self.owner,
            write: Vec::new(),
            read,
        }
    }
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

/// The permissions of a session's token for the repositories it may write
/// to, when its agent may do `access` on GitHub
/// (`crate::limits::session_github`). An agent that comments reviews,
/// comments and labels (`pull_requests` and `issues`), and can't push,
/// merge (both `contents`) or change workflows.
pub fn permissions(access: GithubAccess) -> Value {
    match access {
        GithubAccess::Push => json!({
            "contents": "write",
            "pull_requests": "write",
            "issues": "write",
            "workflows": "write",
            "checks": "read",
            "actions": "read",
            "statuses": "read",
            "metadata": "read"
        }),
        GithubAccess::Comment => json!({
            "contents": "read",
            "pull_requests": "write",
            "issues": "write",
            "checks": "read",
            "actions": "read",
            "statuses": "read",
            "metadata": "read"
        }),
        GithubAccess::Read => read_permissions(),
    }
}

/// The permissions of a session's token for the repositories it only reads.
pub fn read_permissions() -> Value {
    json!({
        "contents": "read",
        "pull_requests": "read",
        "issues": "read",
        "checks": "read",
        "actions": "read",
        "statuses": "read",
        "metadata": "read"
    })
}

/// The permissions of the token the control plane resolves a review thread
/// with, for an agent that may comment: GitHub lets only tokens that can
/// write a repository's contents resolve its threads. The token never
/// leaves the control plane.
pub fn resolve_permissions() -> Value {
    json!({
        "contents": "write",
        "pull_requests": "write",
        "metadata": "read"
    })
}

/// The permissions of the token the control plane posts a review with, for
/// a review agent, whose own token only reads: it writes the review, edits
/// fctl's earlier findings and reacts to the pull request, and with
/// `checks`, writes the review's check. Contents stay read only, so it
/// can't push; it never leaves the control plane.
pub fn review_permissions(checks: bool) -> Value {
    let mut p = json!({
        "contents": "read",
        "pull_requests": "write",
        "metadata": "read"
    });
    if checks {
        p["checks"] = json!("write");
    }
    p
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

/// Whether the control plane may ask GitHub about a pull request of `repo`
/// for a session, with the app's installation on its account: only where
/// the session's own tokens reach, which its project uses ([`access`]) and
/// its workspace may use (`scope`, what the workspace may use on the
/// repository's account; `None`: it doesn't work there). Agents report the
/// pull requests they find, and a host, or anything in the sandbox, can
/// claim any pull request at all; GitHub's answer (title, state, head
/// commit, checks) goes to the session's viewers.
pub fn may_look_up(
    primary: Option<&RepoRef>,
    grants: &[RepoGrant],
    scope: Option<&Scope>,
    repo: &RepoRef,
) -> bool {
    access(primary, grants, repo).is_some() && scope.is_some_and(|s| s.allows(&repo.name))
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

/// A GitHub account's login, as people type it (`@hyperstructure`, or its
/// github.com URL): letters, digits and single dashes, at most 39.
pub fn account(input: &str) -> Result<String> {
    let mut s = input.trim().trim_start_matches('@');
    for prefix in ["https://github.com/", "http://github.com/", "github.com/"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest;
            break;
        }
    }
    let s = s.trim_end_matches('/');
    let ok = !s.is_empty()
        && s.len() <= 39
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--");
    if ok {
        Ok(s.to_owned())
    } else {
        Err(ApiError::validation(
            "account",
            "Use a GitHub account's name, like hyperstructure",
        ))
    }
}

/// The repositories of `account` a workspace's connection is narrowed to,
/// by name (`owner/name` works too, for `account`): without repeats, at
/// least one and at most [`MAX_REPOS`].
pub fn scope_names(account: &str, names: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for raw in names {
        let raw = raw.trim();
        let full = if raw.contains('/') {
            raw.to_owned()
        } else {
            format!("{account}/{raw}")
        };
        let r = RepoRef::parse(&full).ok_or_else(|| {
            ApiError::validation("repos", format!("{raw} isn't a repository name"))
        })?;
        if !r.owner.eq_ignore_ascii_case(account) {
            return Err(ApiError::validation(
                "repos",
                format!("{} isn't a repository of {account}", r.full_name()),
            ));
        }
        if !out.iter().any(|n| n.eq_ignore_ascii_case(&r.name)) {
            out.push(r.name);
        }
    }
    if out.is_empty() {
        return Err(ApiError::validation(
            "repos",
            "Name at least one repository, or allow every repository of the account",
        ));
    }
    if out.len() > MAX_REPOS {
        return Err(ApiError::validation(
            "repos",
            format!("Name at most {MAX_REPOS} repositories"),
        ));
    }
    Ok(out)
}

/// The repositories of a GitHub account that a workspace's projects may
/// use: every one the GitHub App reaches there, or these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    All,
    /// By name, without the owner.
    Repos(Vec<String>),
}

impl Scope {
    /// From a connection's stored list (`None`: every repository).
    pub fn of(repos: Option<Vec<String>>) -> Self {
        repos.map_or(Self::All, Self::Repos)
    }

    /// Whether `name` is one of its repositories.
    pub fn allows(&self, name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Repos(names) => names.iter().any(|n| n.eq_ignore_ascii_case(name)),
        }
    }
}

/// Checks that a workspace may use `owner/name` (`name`: `None` for every
/// repository of `owner`, which a scope narrows rather than refuses).
/// `scope` is what the workspace may use on `owner`; `None` when it doesn't
/// work in that account.
pub fn check_scope(scope: Option<&Scope>, owner: &str, name: Option<&str>) -> Result<()> {
    let Some(scope) = scope else {
        return Err(ApiError::validation(
            "repo",
            format!(
                "This workspace doesn't work in {owner} on GitHub. Its admins connect the account under the workspace's GitHub tab."
            ),
        ));
    };
    match name {
        Some(name) if !scope.allows(name) => Err(ApiError::validation(
            "repo",
            format!(
                "{owner}/{name} isn't one of the repositories this workspace may use. Its admins add it by connecting {owner} again, under the workspace's GitHub tab."
            ),
        )),
        _ => Ok(()),
    }
}

/// A session's token plan within what its workspace may use on the account
/// (`scope`; `None`: nothing, the workspace doesn't work there). Listed
/// repositories outside it are left out and returned (`*` for an
/// owner-wide grant), and an owner-wide grant reads only the scope's
/// repositories.
pub fn narrow(plan: TokenPlan, scope: Option<&Scope>) -> (TokenPlan, Vec<String>) {
    let mut refused = Vec::new();
    let Some(scope) = scope else {
        refused.extend(plan.write);
        match plan.read {
            ReadScope::None => {}
            ReadScope::Repos(names) => refused.extend(names),
            ReadScope::All => refused.push("*".to_owned()),
        }
        let none = TokenPlan {
            owner: plan.owner,
            write: Vec::new(),
            read: ReadScope::None,
        };
        return (none, refused);
    };
    let mut keep = |names: Vec<String>| -> Vec<String> {
        let (ok, out): (Vec<String>, Vec<String>) =
            names.into_iter().partition(|n| scope.allows(n));
        refused.extend(out);
        ok
    };
    let write = keep(plan.write);
    let read = match (plan.read, scope) {
        (ReadScope::None, _) => ReadScope::None,
        (ReadScope::All, Scope::All) => ReadScope::All,
        (ReadScope::All, Scope::Repos(names)) => {
            let rest: Vec<String> = names
                .iter()
                .filter(|n| !write.iter().any(|w| w.eq_ignore_ascii_case(n)))
                .cloned()
                .collect();
            if rest.is_empty() {
                ReadScope::None
            } else {
                ReadScope::Repos(rest)
            }
        }
        (ReadScope::Repos(names), _) => {
            let names = keep(names);
            if names.is_empty() {
                ReadScope::None
            } else {
                ReadScope::Repos(names)
            }
        }
    };
    (
        TokenPlan {
            owner: plan.owner,
            write,
            read,
        },
        refused,
    )
}

/// A project's repositories in a few words, as lists show them: its
/// primary one and how many more its agents may use
/// (`hyperstructure/fctl (+3 more)`), or `(+entire org)` when they may read
/// every repository of its account. `None` when it has none.
pub fn summary(primary: Option<&RepoRef>, others: u32, all_of: Option<&str>) -> Option<String> {
    match (primary, all_of, others) {
        (Some(p), Some(_), _) => Some(format!("{} (+entire org)", p.full_name())),
        (Some(p), None, 0) => Some(p.full_name()),
        (Some(p), None, n) => Some(format!("{} (+{n} more)", p.full_name())),
        (None, Some(owner), _) => Some(format!("{owner} (entire org)")),
        (None, None, 0) => None,
        (None, None, 1) => Some("1 repository".to_owned()),
        (None, None, n) => Some(format!("{n} repositories")),
    }
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
    fn agents_that_read_get_one_token_for_every_repository() {
        let plan = |write: &[&str], read| TokenPlan {
            owner: "acme".into(),
            write: write.iter().map(|w| w.to_string()).collect(),
            read,
        };
        let names = |n: &[&str]| ReadScope::Repos(n.iter().map(|w| w.to_string()).collect());
        assert_eq!(
            plan(&["web", "lib"], names(&["docs"])).reading_only(),
            plan(&[], names(&["web", "lib", "docs"]))
        );
        assert_eq!(
            plan(&["web"], ReadScope::None).reading_only(),
            plan(&[], names(&["web"]))
        );
        assert_eq!(
            plan(&["web"], ReadScope::All).reading_only(),
            plan(&[], ReadScope::All)
        );
        assert_eq!(
            plan(&[], ReadScope::None).reading_only(),
            plan(&[], ReadScope::None)
        );
    }

    #[test]
    fn agents_that_comment_or_read_get_tokens_that_cannot_push() {
        let write = |access| {
            let p = permissions(access);
            ["contents", "workflows"]
                .iter()
                .any(|k| p.get(k).and_then(Value::as_str) == Some("write"))
        };
        assert!(write(GithubAccess::Push));
        assert!(!write(GithubAccess::Comment));
        assert!(!write(GithubAccess::Read));
        let comment = permissions(GithubAccess::Comment);
        for k in ["pull_requests", "issues"] {
            assert_eq!(comment[k], "write", "{k}");
        }
        let read = permissions(GithubAccess::Read);
        assert!(
            read.as_object()
                .is_some_and(|o| o.values().all(|v| v == "read"))
        );
        assert_eq!(resolve_permissions()["contents"], "write");
    }

    #[test]
    fn pull_requests_are_looked_up_only_where_the_sessions_tokens_reach() {
        let primary = r("acme/web");
        let grants = [
            g("acme/lib", RepoAccess::Read),
            g("acme/*", RepoAccess::Read),
        ];
        let some = Scope::Repos(vec!["web".into(), "lib".into(), "docs".into()]);
        let look = |scope: Option<&Scope>, repo: &str| {
            may_look_up(Some(&primary), &grants, scope, &r(repo))
        };
        // The primary repository, a listed one, and one the owner-wide grant
        // reads, within what the workspace may use.
        assert!(look(Some(&some), "acme/web"));
        assert!(look(Some(&some), "ACME/Lib"));
        assert!(look(Some(&some), "acme/docs"));
        // The account's other repositories, which the workspace may not use.
        assert!(!look(Some(&some), "acme/secret"));
        assert!(look(Some(&Scope::All), "acme/secret"));
        // Another account, wherever the app is installed.
        assert!(!look(Some(&Scope::All), "victim/private"));
        // An account the workspace doesn't work in.
        assert!(!look(None, "acme/web"));
        // A project without repositories looks nothing up.
        assert!(!may_look_up(None, &[], Some(&Scope::All), &r("acme/web")));
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

    #[test]
    fn workspaces_use_the_repositories_of_their_scope() {
        let all = Scope::All;
        let some = Scope::Repos(vec!["web".into(), "Design".into()]);
        assert!(check_scope(Some(&all), "acme", Some("anything")).is_ok());
        assert!(check_scope(Some(&some), "acme", Some("design")).is_ok());
        assert!(check_scope(Some(&some), "acme", Some("secret")).is_err());
        // An owner-wide grant is narrowed to the scope, not refused.
        assert!(check_scope(Some(&some), "acme", None).is_ok());
        assert!(check_scope(None, "acme", Some("web")).is_err());
    }

    #[test]
    fn tokens_stay_within_the_workspaces_scope() {
        let plan = TokenPlan {
            owner: "acme".into(),
            write: vec!["web".into(), "secret".into()],
            read: ReadScope::All,
        };
        // Every repository: nothing changes.
        assert_eq!(
            narrow(plan.clone(), Some(&Scope::All)),
            (plan.clone(), vec![])
        );
        // Some: the others go, and reading every repository reads the
        // scope's other ones.
        let some = Scope::Repos(vec!["web".into(), "design".into()]);
        assert_eq!(
            narrow(plan.clone(), Some(&some)),
            (
                TokenPlan {
                    owner: "acme".into(),
                    write: vec!["web".into()],
                    read: ReadScope::Repos(vec!["design".into()]),
                },
                vec!["secret".to_owned()]
            )
        );
        let listed = TokenPlan {
            read: ReadScope::Repos(vec!["design".into(), "infra".into()]),
            ..plan.clone()
        };
        assert_eq!(
            narrow(listed, Some(&some)).0.read,
            ReadScope::Repos(vec!["design".into()])
        );
        // The workspace doesn't work in the account: no token at all.
        let (none, refused) = narrow(plan, None);
        assert!(none.write.is_empty());
        assert_eq!(none.read, ReadScope::None);
        assert_eq!(refused, ["web", "secret", "*"]);
    }

    #[test]
    fn accounts_and_scopes_as_people_type_them() {
        assert_eq!(
            account("hyperstructure").ok(),
            Some("hyperstructure".into())
        );
        assert_eq!(account(" @ada-lovelace ").ok(), Some("ada-lovelace".into()));
        assert_eq!(
            account("https://github.com/side-project/").ok(),
            Some("side-project".into())
        );
        assert!(account("-ada").is_err());
        assert!(account("ada--x").is_err());
        assert!(account("a/b").is_err());
        assert!(account("").is_err());
        let names = |v: &[&str]| -> Vec<String> { v.iter().map(|s| (*s).to_owned()).collect() };
        assert_eq!(
            scope_names("acme", &names(&["web", "acme/Design", "WEB"])).ok(),
            Some(names(&["web", "Design"]))
        );
        assert!(scope_names("acme", &names(&["other/web"])).is_err());
        assert!(scope_names("acme", &names(&[])).is_err());
        assert!(scope_names("acme", &names(&["../x"])).is_err());
    }

    #[test]
    fn summaries_count_the_other_repositories() {
        let fctl = r("hyperstructure/fctl");
        assert_eq!(
            summary(Some(&fctl), 0, None).as_deref(),
            Some("hyperstructure/fctl")
        );
        assert_eq!(
            summary(Some(&fctl), 3, None).as_deref(),
            Some("hyperstructure/fctl (+3 more)")
        );
        // Named ones are in the org too.
        assert_eq!(
            summary(Some(&fctl), 2, Some("hyperstructure")).as_deref(),
            Some("hyperstructure/fctl (+entire org)")
        );
        assert_eq!(
            summary(None, 0, Some("hyperstructure")).as_deref(),
            Some("hyperstructure (entire org)")
        );
        assert_eq!(summary(None, 1, None).as_deref(), Some("1 repository"));
        assert_eq!(summary(None, 2, None).as_deref(), Some("2 repositories"));
        assert_eq!(summary(None, 0, None), None);
    }
}
