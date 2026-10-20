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

use crate::repos::{MAX_REPOS, Scope};

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
    /// They are an admin of more of its repositories than a connection
    /// lists, but not of all of them.
    TooMany,
}

impl Refusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoInstallation => "no_installation",
            Self::NoAdmin => "no_admin",
            Self::NotAdmin => "not_admin",
            Self::Unreachable => "unreachable",
            Self::TooMany => "too_many",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "no_installation" => Some(Self::NoInstallation),
            "no_admin" => Some(Self::NoAdmin),
            "not_admin" => Some(Self::NotAdmin),
            "unreachable" => Some(Self::Unreachable),
            "too_many" => Some(Self::TooMany),
            _ => None,
        }
    }
}

/// What a workspace gets of an account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub scope: Scope,
    /// Every repository was asked for, and only these are the person's to
    /// give.
    pub narrowed: bool,
}

/// What the person may connect of the account, `asked`: every repository
/// (`None`) or these, as [`crate::repos::scope_names`] checked them. Every
/// repository they can't have becomes the ones they are an admin of; named
/// ones they aren't an admin of are refused, with their names.
pub fn grant(
    standing: &Standing,
    asked: Option<&[String]>,
) -> Result<Grant, (Refusal, Vec<String>)> {
    let Some(names) = asked else {
        let mut theirs: Vec<String> = standing
            .seen
            .iter()
            .filter(|r| r.admin)
            .map(|r| r.name.clone())
            .collect();
        let of_all =
            standing.reaches_all && standing.reached > 0 && theirs.len() >= standing.reached;
        if standing.own || of_all {
            return Ok(Grant {
                scope: Scope::All,
                narrowed: false,
            });
        }
        if theirs.is_empty() {
            return Err((Refusal::NoAdmin, Vec::new()));
        }
        if theirs.len() > MAX_REPOS {
            return Err((Refusal::TooMany, Vec::new()));
        }
        theirs.sort_by_key(|n| n.to_lowercase());
        return Ok(Grant {
            scope: Scope::Repos(theirs),
            narrowed: true,
        });
    };
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
    Ok(Grant {
        scope: Scope::Repos(repos),
        narrowed: false,
    })
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

    fn all() -> Result<Grant, (Refusal, Vec<String>)> {
        Ok(Grant {
            scope: Scope::All,
            narrowed: false,
        })
    }

    #[test]
    fn people_connect_every_repository_of_their_own_account() {
        let mine = Standing {
            own: true,
            reaches_all: false,
            reached: 2,
            seen: seen(&[("site", true), ("notes", true)]),
        };
        assert_eq!(grant(&mine, None), all());
        // Also before the app reaches any of it.
        let empty = Standing {
            own: true,
            ..Standing::default()
        };
        assert_eq!(grant(&empty, None), all());
    }

    #[test]
    fn admins_of_every_repository_of_an_organization_connect_all_of_it() {
        // As its owners are: the app reaches all of it, and they administer
        // every repository.
        let owner = Standing {
            own: false,
            reaches_all: true,
            reached: 3,
            seen: seen(&[("web", true), ("api", true), ("design", true)]),
        };
        assert_eq!(grant(&owner, None), all());
    }

    #[test]
    fn everyone_else_gets_the_repositories_they_are_an_admin_of() {
        // A member who administers some of them.
        let member = Standing {
            own: false,
            reaches_all: true,
            reached: 3,
            seen: seen(&[("web", true), ("api", false), ("Design", true)]),
        };
        assert_eq!(
            grant(&member, None),
            Ok(Grant {
                scope: Scope::Repos(names(&["Design", "web"])),
                narrowed: true,
            })
        );
        // The app reaches only some of the organization: what it reaches
        // later may not be theirs, so even an owner gets the ones it
        // reaches now.
        let selected = Standing {
            own: false,
            reaches_all: false,
            reached: 2,
            seen: seen(&[("web", true), ("api", true)]),
        };
        assert_eq!(
            grant(&selected, None),
            Ok(Grant {
                scope: Scope::Repos(names(&["api", "web"])),
                narrowed: true,
            })
        );
        // They see fewer than it reaches.
        let partial = Standing {
            own: false,
            reaches_all: true,
            reached: 5,
            seen: seen(&[("web", true)]),
        };
        assert_eq!(
            grant(&partial, None),
            Ok(Grant {
                scope: Scope::Repos(names(&["web"])),
                narrowed: true,
            })
        );
    }

    #[test]
    fn people_who_administer_none_of_it_connect_nothing() {
        let reader = Standing {
            own: false,
            reaches_all: true,
            reached: 2,
            seen: seen(&[("web", false), ("api", false)]),
        };
        assert_eq!(grant(&reader, None), Err((Refusal::NoAdmin, Vec::new())));
        // Nobody can tell an empty organization's owners from strangers.
        let empty = Standing {
            own: false,
            reaches_all: true,
            reached: 0,
            seen: Vec::new(),
        };
        assert_eq!(grant(&empty, None), Err((Refusal::NoAdmin, Vec::new())));
    }

    #[test]
    fn more_than_a_connection_lists_is_refused() {
        let many: Vec<Seen> = (0..=MAX_REPOS)
            .map(|i| Seen {
                name: format!("repo-{i}"),
                admin: true,
            })
            .collect();
        let standing = Standing {
            own: false,
            reaches_all: true,
            reached: MAX_REPOS + 5,
            seen: many,
        };
        assert_eq!(grant(&standing, None), Err((Refusal::TooMany, Vec::new())));
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
            grant(&member, Some(&names(&["web", "design"]))),
            Ok(Grant {
                scope: Scope::Repos(names(&["Web", "design"])),
                narrowed: false,
            })
        );
        assert_eq!(
            grant(&member, Some(&names(&["web", "api"]))),
            Err((Refusal::NotAdmin, names(&["api"])))
        );
        assert_eq!(
            grant(&member, Some(&names(&["web", "secret", "api"]))),
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
            grant(&mine, Some(&names(&["notes"]))),
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
            Refusal::TooMany,
        ] {
            assert_eq!(Refusal::parse(r.as_str()), Some(r));
        }
        assert_eq!(Refusal::parse("nope"), None);
    }
}
