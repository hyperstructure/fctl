//! What a workspace's admin may connect of a GitHub account, as GitHub
//! tells fctl when they sign in to it (docs/architecture.md#github-accounts).
//!
//! The instance's GitHub App mints tokens for every account it is installed
//! on, and a workspace that works in an account lets its people's agents
//! write to the account's repositories. So a workspace gets only what the
//! admin who connects it could give people on GitHub: repositories they are
//! an admin of. Every repository of the account, also those the app reaches
//! there later, takes their own account, or an organization whose every
//! repository the app reaches and they are an admin of, as its owners are.
//!
//! The admin chooses from what the sign-in showed them ([`offer`],
//! [`choose`]), or names the repositories before it ([`grant`]).

use crate::repos::{MAX_REPOS, Scope};
use crate::validate::Result;
use fctl_proto::ApiError;

/// A repository the app's installation reaches, as the person sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    /// Its name, as GitHub spells it.
    pub name: String,
    /// They are an admin of it.
    pub admin: bool,
}

/// What GitHub says about the person and the app's installation on the
/// account.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Standing {
    /// It is installed on their own account.
    pub own: bool,
    /// It reaches every repository of the account (its repository
    /// selection is `all`).
    pub reaches_all: bool,
    /// How many repositories it reaches.
    pub reached: usize,
    /// The ones they can see.
    pub seen: Vec<Seen>,
}

/// Why someone may not connect what they asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// GitHub shows them no installation of the app there: it isn't
    /// installed, or reaches no repository they can see. Either way the
    /// same, so that nobody learns where the app is installed.
    NoInstallation,
    /// They are an admin of none of the repositories the app reaches there.
    NoAdmin,
    /// They aren't an admin of some of the repositories they named.
    NotAdmin,
    /// The app doesn't reach some of the repositories they named, or they
    /// can't see them.
    Unreachable,
}

impl Refusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoInstallation => "no_installation",
            Self::NoAdmin => "no_admin",
            Self::NotAdmin => "not_admin",
            Self::Unreachable => "unreachable",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "no_installation" => Some(Self::NoInstallation),
            "no_admin" => Some(Self::NoAdmin),
            "not_admin" => Some(Self::NotAdmin),
            "unreachable" => Some(Self::Unreachable),
            _ => None,
        }
    }
}

/// What the person may connect of the account, from which they choose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    /// Every repository, also those the app reaches there later: their own
    /// account, or an organization whose every repository the app reaches
    /// and they are an admin of.
    pub all: bool,
    /// The repositories the app reaches that they are an admin of, as
    /// GitHub spells them, in alphabetical order.
    pub repos: Vec<String>,
    /// How many more it reaches that they can see but aren't an admin of.
    pub others: usize,
}

/// What the person may choose from of the account: nothing when it isn't
/// their own and they are an admin of none of its repositories the app
/// reaches.
pub fn offer(standing: &Standing) -> std::result::Result<Offer, Refusal> {
    let mut repos: Vec<String> = standing
        .seen
        .iter()
        .filter(|r| r.admin)
        .map(|r| r.name.clone())
        .collect();
    repos.sort_by_key(|n| n.to_lowercase());
    let others = standing.seen.len() - repos.len();
    let of_all = standing.reaches_all && standing.reached > 0 && repos.len() >= standing.reached;
    let all = standing.own || of_all;
    if !all && repos.is_empty() {
        return Err(Refusal::NoAdmin);
    }
    Ok(Offer { all, repos, others })
}

/// What a workspace gets of what was offered: every repository (`None`),
/// where the offer has it, or the ones `chosen`, as the offer spells them.
pub fn choose(offer: &Offer, chosen: Option<&[String]>) -> Result<Scope> {
    let Some(names) = chosen else {
        if offer.all {
            return Ok(Scope::All);
        }
        return Err(ApiError::validation(
            "repos",
            "Choose the repositories: you may connect the ones you're an admin of, not every one",
        ));
    };
    let mut repos: Vec<String> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();
    for name in names.iter().map(|n| n.trim()) {
        match offer.repos.iter().find(|r| r.eq_ignore_ascii_case(name)) {
            Some(r) if !repos.contains(r) => repos.push(r.clone()),
            Some(_) => {}
            None => missing.push(name),
        }
    }
    if !missing.is_empty() {
        return Err(ApiError::validation(
            "repos",
            format!(
                "You may connect only repositories the app reaches that you're an admin of, which {} isn't",
                missing.join(", ")
            ),
        ));
    }
    if repos.is_empty() {
        return Err(ApiError::validation(
            "repos",
            "Choose at least one repository",
        ));
    }
    if repos.len() > MAX_REPOS {
        return Err(ApiError::validation(
            "repos",
            format!("Choose at most {MAX_REPOS} repositories"),
        ));
    }
    repos.sort_by_key(|n| n.to_lowercase());
    Ok(Scope::Repos(repos))
}

/// What the person may connect of the repositories they named before
/// signing in, as [`crate::repos::scope_names`] checked them: each one the
/// app reaches that they see and are an admin of, as GitHub spells it.
/// Otherwise the refusal, with the names it is for.
pub fn grant(
    standing: &Standing,
    names: &[String],
) -> std::result::Result<Scope, (Refusal, Vec<String>)> {
    let mut repos = Vec::new();
    let (mut unreachable, mut not_admin) = (Vec::new(), Vec::new());
    for name in names {
        match standing
            .seen
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(name))
        {
            None => unreachable.push(name.clone()),
            Some(r) if !r.admin => not_admin.push(r.name.clone()),
            Some(r) => repos.push(r.name.clone()),
        }
    }
    if !unreachable.is_empty() {
        return Err((Refusal::Unreachable, unreachable));
    }
    if !not_admin.is_empty() {
        return Err((Refusal::NotAdmin, not_admin));
    }
    Ok(Scope::Repos(repos))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(repos: &[(&str, bool)]) -> Vec<Seen> {
        repos
            .iter()
            .map(|(name, admin)| Seen {
                name: (*name).to_owned(),
                admin: *admin,
            })
            .collect()
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    fn offered(all: bool, repos: &[&str], others: usize) -> std::result::Result<Offer, Refusal> {
        Ok(Offer {
            all,
            repos: names(repos),
            others,
        })
    }

    #[test]
    fn people_may_connect_every_repository_of_their_own_account() {
        // Also when the app reaches only some of it: the rest is theirs too.
        let mine = Standing {
            own: true,
            reaches_all: false,
            reached: 2,
            seen: seen(&[("site", true), ("notes", true)]),
        };
        assert_eq!(offer(&mine), offered(true, &["notes", "site"], 0));
        // And before the app reaches any of it.
        let empty = Standing {
            own: true,
            ..Standing::default()
        };
        assert_eq!(offer(&empty), offered(true, &[], 0));
    }

    #[test]
    fn admins_of_every_repository_of_an_organization_may_connect_all_of_it() {
        // As its owners are: the app reaches all of it, and they administer
        // every repository.
        let owner = Standing {
            own: false,
            reaches_all: true,
            reached: 3,
            seen: seen(&[("web", true), ("api", true), ("design", true)]),
        };
        assert_eq!(offer(&owner), offered(true, &["api", "design", "web"], 0));
    }

    #[test]
    fn everyone_else_may_connect_the_repositories_they_are_an_admin_of() {
        // A member who administers some of them.
        let member = Standing {
            own: false,
            reaches_all: true,
            reached: 3,
            seen: seen(&[("web", true), ("api", false), ("Design", true)]),
        };
        assert_eq!(offer(&member), offered(false, &["Design", "web"], 1));
        // The app reaches only some of the organization: what it reaches
        // later may not be theirs, so even an owner chooses among the ones
        // it reaches now.
        let selected = Standing {
            own: false,
            reaches_all: false,
            reached: 2,
            seen: seen(&[("web", true), ("api", true)]),
        };
        assert_eq!(offer(&selected), offered(false, &["api", "web"], 0));
        // They see fewer than it reaches.
        let partial = Standing {
            own: false,
            reaches_all: true,
            reached: 5,
            seen: seen(&[("web", true)]),
        };
        assert_eq!(offer(&partial), offered(false, &["web"], 0));
        // More than a connection lists: they choose some.
        let many = Standing {
            own: false,
            reaches_all: true,
            reached: MAX_REPOS + 5,
            seen: (0..=MAX_REPOS)
                .map(|i| Seen {
                    name: format!("repo-{i:03}"),
                    admin: true,
                })
                .collect(),
        };
        let o = offer(&many).map(|o| (o.all, o.repos.len()));
        assert_eq!(o, Ok((false, MAX_REPOS + 1)));
    }

    #[test]
    fn people_who_administer_none_of_it_are_offered_nothing() {
        let reader = Standing {
            own: false,
            reaches_all: true,
            reached: 2,
            seen: seen(&[("web", false), ("api", false)]),
        };
        assert_eq!(offer(&reader), Err(Refusal::NoAdmin));
        // Nobody can tell an empty organization's owners from strangers.
        let empty = Standing {
            own: false,
            reaches_all: true,
            reached: 0,
            seen: Vec::new(),
        };
        assert_eq!(offer(&empty), Err(Refusal::NoAdmin));
    }

    #[test]
    fn every_repository_is_chosen_only_where_it_was_offered() {
        let all = Offer {
            all: true,
            repos: names(&["site", "notes"]),
            others: 0,
        };
        assert_eq!(choose(&all, None), Ok(Scope::All));
        let some = Offer {
            all: false,
            ..all.clone()
        };
        let refused = choose(&some, None).map_err(|e| e.field);
        assert_eq!(refused, Err(Some("repos".into())));
    }

    #[test]
    fn chosen_repositories_come_from_the_offer() {
        let o = Offer {
            all: true,
            repos: names(&["Web", "api", "design"]),
            others: 2,
        };
        // As the offer spells them, once each, in alphabetical order.
        assert_eq!(
            choose(&o, Some(&names(&[" web", "design", "WEB"]))),
            Ok(Scope::Repos(names(&["design", "Web"])))
        );
        // Not what it didn't offer, nor nothing at all.
        let secret = choose(&o, Some(&names(&["web", "secret"]))).map_err(|e| e.message);
        assert!(secret.is_err_and(|m| m.contains("secret")));
        assert!(choose(&o, Some(&[])).is_err());
        // At most as many as a connection lists.
        let many = Offer {
            all: false,
            repos: (0..=MAX_REPOS).map(|i| format!("repo-{i:03}")).collect(),
            others: 0,
        };
        assert!(choose(&many, Some(&many.repos)).is_err());
        assert_eq!(
            choose(&many, Some(&many.repos[..MAX_REPOS])),
            Ok(Scope::Repos(many.repos[..MAX_REPOS].to_vec()))
        );
    }

    #[test]
    fn named_repositories_must_be_reached_and_administered() {
        let member = Standing {
            own: false,
            reaches_all: true,
            reached: 3,
            seen: seen(&[("Web", true), ("api", false), ("design", true)]),
        };
        // As GitHub spells them.
        assert_eq!(
            grant(&member, &names(&["web", "design"])),
            Ok(Scope::Repos(names(&["Web", "design"])))
        );
        assert_eq!(
            grant(&member, &names(&["web", "api"])),
            Err((Refusal::NotAdmin, names(&["api"])))
        );
        assert_eq!(
            grant(&member, &names(&["web", "secret", "api"])),
            Err((Refusal::Unreachable, names(&["secret"])))
        );
        // Their own account's, too, only where the app reaches.
        let mine = Standing {
            own: true,
            reaches_all: false,
            reached: 1,
            seen: seen(&[("site", true)]),
        };
        assert_eq!(
            grant(&mine, &names(&["notes"])),
            Err((Refusal::Unreachable, names(&["notes"])))
        );
    }

    #[test]
    fn refusals_round_trip() {
        for r in [
            Refusal::NoInstallation,
            Refusal::NoAdmin,
            Refusal::NotAdmin,
            Refusal::Unreachable,
        ] {
            assert_eq!(Refusal::parse(r.as_str()), Some(r));
        }
        assert_eq!(Refusal::parse("nope"), None);
    }
}
