//! How people sign in: with a password, a passkey, or a provider such as
//! Google; what a sign-in keeps of how it was made; and the rules that
//! require a provider of the people at some domains ("acme.com and
//! acme.dev: Google only", `InstanceSettings::sign_in`).
//!
//! A rule covers an account when any of its addresses is at one of the
//! rule's domains: an address it connected counts as much as its primary
//! one, so that connecting a personal address is no way around it. A
//! covered account signs in only with the rule's provider, as an address at
//! one of the rule's domains that the provider vouches for (Google: where it
//! hosts the address, `crate::google::vouches_for`), and with a passkey
//! alone only where the rule allows that. Sign-ins made another way stop
//! working, those made before the rule included: each token keeps how its
//! sign-in was made ([`SignedIn`]), and one from before fctl kept that
//! meets no rule. The primary instance owner keeps every way in, so that the
//! instance has one while the provider is down.
//!
//! Google is the one provider today. Another (an OpenID Connect issuer an
//! admin adds) gets an id of its own, which rules, tokens and identities
//! name as they name `google`, and says which addresses it vouches for.

use fctl_proto::api::{SignInPerson, SignInPreview, SignInRule, SignInYou};
use fctl_proto::{ApiError, UserId};
use std::collections::BTreeSet;

/// Google's id, as rules, tokens and identities (`identities.provider`)
/// name it.
pub const GOOGLE: &str = "google";

/// The most domains the rules name in all.
pub const MAX_DOMAINS: usize = 100;

/// How a sign-in was made, as its token keeps it (`tokens.auth_method`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Method {
    /// A password, with or without a second factor; a reset link's new one
    /// too.
    Password,
    /// A passkey alone, not as the second factor of another sign-in.
    Passkey,
    /// A provider's sign-in: its id (`google`).
    Provider(String),
}

impl Method {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Password => "password",
            Self::Passkey => "passkey",
            Self::Provider(id) => id,
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "password" => Self::Password,
            "passkey" => Self::Passkey,
            id => Self::Provider(id.to_owned()),
        }
    }
}

/// How a sign-in was made: what its browser session keeps, and what a CLI
/// sign-in it approves and an access token made with it inherit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignedIn {
    /// `None` for a sign-in from before fctl kept how, which meets no rule.
    pub method: Option<Method>,
    /// For a provider's: the domain of the address it vouched for.
    pub domain: Option<String>,
}

impl SignedIn {
    pub fn password() -> Self {
        Self {
            method: Some(Method::Password),
            domain: None,
        }
    }

    pub fn passkey() -> Self {
        Self {
            method: Some(Method::Passkey),
            domain: None,
        }
    }

    /// A sign-in with `provider`, which vouched for `address` (or for no
    /// address at all).
    pub fn provider(provider: &str, address: Option<&str>) -> Self {
        Self {
            method: Some(Method::Provider(provider.to_owned())),
            domain: address.and_then(domain_of),
        }
    }

    /// As a token keeps it (`tokens.auth_method`, `tokens.auth_domain`).
    pub fn read(method: Option<&str>, domain: Option<&str>) -> Self {
        Self {
            method: method.filter(|m| !m.is_empty()).map(Method::parse),
            domain: domain.filter(|d| !d.is_empty()).map(str::to_owned),
        }
    }

    /// What `tokens.auth_method` keeps.
    pub fn method_str(&self) -> Option<&str> {
        self.method.as_ref().map(Method::as_str)
    }
}

/// The domain of an address, lowercase.
pub fn domain_of(address: &str) -> Option<String> {
    let (local, domain) = address.trim().rsplit_once('@')?;
    let domain = domain.trim().to_ascii_lowercase();
    (!local.is_empty() && !domain.is_empty()).then_some(domain)
}

/// A domain as rules keep it, from what an admin typed: `acme.com`,
/// `@acme.com` or `*@acme.com`, in any case.
pub fn domain(input: &str) -> Result<String, ApiError> {
    let typed = input.trim().to_ascii_lowercase();
    let domain = typed
        .strip_prefix("*@")
        .or_else(|| typed.strip_prefix('@'))
        .unwrap_or(&typed)
        .trim_end_matches('.');
    // As the allowlist checks domains: a sample address on it.
    crate::validate::email(&format!("x@{domain}")).map_err(|_| {
        ApiError::validation(
            "domains",
            format!("{} isn't a domain: give one like acme.com", input.trim()),
        )
    })?;
    Ok(domain.to_owned())
}

/// `rules` as the instance keeps them, from what an admin saves: each with
/// its domains checked, lowercase, sorted and once each, at least one
/// each, a domain in one rule at most, [`MAX_DOMAINS`] in all, and a
/// provider among `providers`, those the instance has set up.
pub fn check_rules(rules: &[SignInRule], providers: &[&str]) -> Result<Vec<SignInRule>, ApiError> {
    let mut seen = BTreeSet::new();
    let mut checked = Vec::with_capacity(rules.len());
    for rule in rules {
        if !providers.contains(&rule.provider.as_str()) {
            return Err(ApiError::validation(
                "provider",
                if rule.provider == GOOGLE {
                    "Google sign-in isn't set up on this instance: its deployment sets GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET first".to_owned()
                } else {
                    format!("This instance has no sign-in provider {}", rule.provider)
                },
            ));
        }
        let mut domains = BTreeSet::new();
        for typed in &rule.domains {
            domains.insert(domain(typed)?);
        }
        if domains.is_empty() {
            return Err(ApiError::validation(
                "domains",
                "Name the domains whose people sign in this way, like acme.com",
            ));
        }
        for d in &domains {
            if !seen.insert(d.clone()) {
                return Err(ApiError::validation(
                    "domains",
                    format!("{d} is in two rules: a domain's people sign in one way"),
                ));
            }
        }
        checked.push(SignInRule {
            domains: domains.into_iter().collect(),
            provider: rule.provider.clone(),
            passkeys: rule.passkeys,
        });
    }
    if seen.len() > MAX_DOMAINS {
        return Err(ApiError::validation(
            "domains",
            format!("The rules can name {MAX_DOMAINS} domains at most"),
        ));
    }
    Ok(checked)
}

/// `rules` with `domains` requiring `provider`, and passkeys alone too
/// where `passkeys`: each domain moves out of the rule it was in, into the
/// rule that says the same of its people, or a new one. Rules left without
/// domains go. Domains as admins type them are fine: [`check_rules`] reads
/// them.
pub fn with_required(
    rules: &[SignInRule],
    domains: &[String],
    provider: &str,
    passkeys: bool,
) -> Vec<SignInRule> {
    let mut next = without(rules, domains);
    let moved = domains
        .iter()
        .map(|d| domain(d).unwrap_or_else(|_| d.clone()));
    match next
        .iter_mut()
        .find(|r| r.provider == provider && r.passkeys == passkeys)
    {
        Some(rule) => {
            for d in moved {
                if !rule.domains.contains(&d) {
                    rule.domains.push(d);
                }
            }
            rule.domains.sort();
        }
        None => {
            let mut domains: Vec<String> = moved.collect();
            domains.sort();
            domains.dedup();
            next.push(SignInRule {
                domains,
                provider: provider.to_owned(),
                passkeys,
            });
        }
    }
    next
}

/// `rules` without `domains`: their people sign in as they like again.
/// Rules left without domains go.
pub fn without(rules: &[SignInRule], domains: &[String]) -> Vec<SignInRule> {
    let gone: Vec<String> = domains
        .iter()
        .map(|d| domain(d).unwrap_or_else(|_| d.clone()))
        .collect();
    rules
        .iter()
        .map(|r| SignInRule {
            domains: r
                .domains
                .iter()
                .filter(|d| !gone.contains(d))
                .cloned()
                .collect(),
            ..r.clone()
        })
        .filter(|r| !r.domains.is_empty())
        .collect()
}

/// The rules that cover an account with `addresses`: those with a domain
/// of one of them.
pub fn covering<'a>(rules: &'a [SignInRule], addresses: &[&str]) -> Vec<&'a SignInRule> {
    let domains: Vec<String> = addresses.iter().filter_map(|a| domain_of(a)).collect();
    rules
        .iter()
        .filter(|r| r.domains.iter().any(|d| domains.contains(d)))
        .collect()
}

/// Whether a sign-in made `how` meets `rule`: with its provider, as an
/// address at one of its domains, or with a passkey alone where it allows
/// that.
pub fn meets(rule: &SignInRule, how: &SignedIn) -> bool {
    match &how.method {
        Some(Method::Provider(p)) => {
            *p == rule.provider
                && how
                    .domain
                    .as_ref()
                    .is_some_and(|d| rule.domains.contains(d))
        }
        Some(Method::Passkey) => rule.passkeys,
        Some(Method::Password) | None => false,
    }
}

/// What a rule requires of an account a sign-in didn't meet: the provider,
/// and the account's domains it covers, which name the addresses to sign in
/// as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Required {
    pub provider: String,
    pub domains: Vec<String>,
    pub passkeys: bool,
}

impl Required {
    fn of(rule: &SignInRule, addresses: &[&str]) -> Self {
        let mine: BTreeSet<String> = addresses.iter().filter_map(|a| domain_of(a)).collect();
        Self {
            provider: rule.provider.clone(),
            domains: rule
                .domains
                .iter()
                .filter(|d| mine.contains(*d))
                .cloned()
                .collect(),
            passkeys: rule.passkeys,
        }
    }
}

/// What the rules require of an account with `addresses`, if any covers
/// it: the first that does. `exempt`: it is the primary instance owner's.
pub fn required(rules: &[SignInRule], addresses: &[&str], exempt: bool) -> Option<Required> {
    if exempt {
        return None;
    }
    covering(rules, addresses)
        .first()
        .map(|r| Required::of(r, addresses))
}

/// Whether a sign-in made `how` lets an account with `addresses` in: it
/// meets one of the rules that cover it, or none does, or the account is
/// the primary instance owner's (`exempt`). Otherwise, what the first of
/// them requires. A sign-in that meets one rule of an account at two
/// organizations' domains lets it in: each vouches for its own people.
pub fn admits(
    rules: &[SignInRule],
    addresses: &[&str],
    exempt: bool,
    how: &SignedIn,
) -> Result<(), Required> {
    if exempt {
        return Ok(());
    }
    let covering = covering(rules, addresses);
    match covering.first() {
        None => Ok(()),
        Some(_) if covering.iter().any(|r| meets(r, how)) => Ok(()),
        Some(first) => Err(Required::of(first, addresses)),
    }
}

/// Whether an instance admin whose role is `actor` may unlink the provider
/// accounts of someone whose role is `target` (`own`: their own). Their own
/// they unlink in Settings, where they keep another way in; nobody the
/// primary instance owner's; and only the primary instance owner an
/// admin's, as with reset links.
pub fn check_unlink(
    actor: fctl_proto::session::InstanceRole,
    target: fctl_proto::session::InstanceRole,
    own: bool,
) -> Result<(), ApiError> {
    use fctl_proto::session::InstanceRole::{Admin, Owner};
    if own {
        return Err(ApiError::conflict(
            "Unlink your own in Settings → Security, where you keep another way in.",
        ));
    }
    match (actor, target) {
        (_, Owner) => Err(ApiError::forbidden(
            "Only the primary instance owner unlinks their own, in Settings",
        )),
        (a, Admin) if a != Owner => Err(ApiError::forbidden(
            "Only the primary instance owner can unlink an admin's sign-in",
        )),
        _ => Ok(()),
    }
}

/// "acme.com", "acme.com and acme.dev", "acme.com, acme.dev and acme.io".
pub fn domains_words(domains: &[String]) -> String {
    match domains {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// An account, as the rules and a change to them concern it.
#[derive(Clone, Debug, Default)]
pub struct Person {
    pub id: String,
    pub name: String,
    /// Its primary address.
    pub email: String,
    pub avatar_url: Option<String>,
    /// All its addresses, the primary one too.
    pub addresses: Vec<String>,
    /// The primary instance owner's.
    pub owner: bool,
    pub password: bool,
    pub passkeys: bool,
    /// The providers whose accounts sign in to it, each with the domain of
    /// the address it vouched for at its last sign-in, if it vouched for
    /// one (`identities.vouched_domain`).
    pub identities: Vec<(String, Option<String>)>,
    /// Its CLI sign-ins and access tokens (`cli`, `pat`) and its admin
    /// tokens (`admin`), each with how the sign-in that made it was made.
    pub tokens: Vec<(String, SignedIn)>,
}

impl Person {
    fn addresses(&self) -> Vec<&str> {
        self.addresses.iter().map(String::as_str).collect()
    }

    /// Whether it has no way to sign in at all: an admin token made it,
    /// and nobody has signed in to it.
    pub fn no_way_in(&self) -> bool {
        !self.password && !self.passkeys && self.identities.is_empty()
    }

    /// Whether an account of the provider of one of `rules`, vouched for as
    /// an address at one of its domains, signs in to it.
    fn linked(&self, rules: &[&SignInRule]) -> bool {
        self.identities.iter().any(|(provider, domain)| {
            let how = SignedIn::read(Some(provider), domain.as_deref());
            rules.iter().any(|r| meets(r, &how))
        })
    }

    /// Its tokens of `kinds` that `before` lets in and `after` doesn't.
    fn tokens_stopped(&self, before: &[SignInRule], after: &[SignInRule], kinds: &[&str]) -> u32 {
        let addresses = self.addresses();
        self.tokens
            .iter()
            .filter(|(kind, how)| {
                kinds.contains(&kind.as_str())
                    && admits(before, &addresses, self.owner, how).is_ok()
                    && admits(after, &addresses, self.owner, how).is_err()
            })
            .count() as u32
    }

    /// Whether its password signs it in under `rules`.
    fn password_works(&self, rules: &[SignInRule]) -> bool {
        self.password && admits(rules, &self.addresses(), self.owner, &SignedIn::password()).is_ok()
    }

    /// Whether a passkey alone signs it in under `rules`.
    fn passkey_works(&self, rules: &[SignInRule]) -> bool {
        self.passkeys && admits(rules, &self.addresses(), self.owner, &SignedIn::passkey()).is_ok()
    }

    pub fn to_api(&self) -> SignInPerson {
        SignInPerson {
            id: UserId(self.id.clone()),
            name: self.name.clone(),
            email: self.email.clone(),
            avatar_url: self.avatar_url.clone(),
        }
    }
}

/// Whom `rule` covers among `people`, the active accounts.
pub fn coverage(rule: &SignInRule, people: &[Person]) -> fctl_proto::api::SignInCoverage {
    let rules = std::slice::from_ref(rule);
    let mut c = fctl_proto::api::SignInCoverage::default();
    for p in people {
        let covering = covering(rules, &p.addresses());
        if covering.is_empty() {
            continue;
        }
        c.people += 1;
        if p.linked(&covering) {
            c.linked += 1;
        }
        if p.password && !p.owner {
            c.passwords += 1;
        }
    }
    c
}

/// A way in that saving `after` in place of `before` takes from someone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lost {
    /// Their password stops signing them in.
    pub password: bool,
    /// Their passkeys stop signing them in alone.
    pub passkeys: bool,
}

/// Whom saving `after` in place of `before` takes a way in from, among
/// `people`, the active accounts: those whose password, or passkeys alone,
/// sign them in now and wouldn't. Never the primary instance owner.
pub fn affected<'a>(
    before: &[SignInRule],
    after: &[SignInRule],
    people: &'a [Person],
) -> Vec<(&'a Person, Lost)> {
    people
        .iter()
        .filter(|p| !p.owner)
        .filter_map(|p| {
            let lost = Lost {
                password: p.password_works(before) && !p.password_works(after),
                passkeys: p.passkey_works(before) && !p.passkey_works(after),
            };
            (lost.password || lost.passkeys).then_some((p, lost))
        })
        .collect()
}

/// What saving `after` in place of `before` would do to `people`, the
/// active accounts: whose password and passkeys stop working, who sign in
/// with the provider already, and what happens to the sign-in of `actor`,
/// the admin who saves it, which was made `how`.
pub fn preview(
    before: &[SignInRule],
    after: &[SignInRule],
    people: &[Person],
    actor: &str,
    how: &SignedIn,
) -> SignInPreview {
    let mut p = SignInPreview {
        covered: 0,
        passwords: Vec::new(),
        password_count: 0,
        passkeys: Vec::new(),
        passkey_count: 0,
        access_tokens: 0,
        admin_tokens: 0,
        linked: 0,
        first: 0,
        released: 0,
        others: 0,
        owner: None,
        you: SignInYou::Outside,
        email: false,
    };
    for person in people {
        let addresses = person.addresses();
        let now = covering(after, &addresses);
        let was = covering(before, &addresses);
        if person.id == actor {
            p.you = you(after, person, how);
        }
        if now.is_empty() {
            p.others += 1;
            if !was.is_empty() && !person.owner {
                p.released += 1;
            }
            continue;
        }
        p.covered += 1;
        if person.owner {
            p.owner = Some(person.to_api());
            continue;
        }
        p.access_tokens += person.tokens_stopped(before, after, &["cli", "pat"]);
        p.admin_tokens += person.tokens_stopped(before, after, &["admin"]);
        if person.linked(&now) {
            p.linked += 1;
        } else if person.no_way_in() {
            p.first += 1;
        }
    }
    for (person, lost) in affected(before, after, people) {
        if lost.password {
            p.password_count += 1;
            if p.passwords.len() < SignInPreview::LIMIT {
                p.passwords.push(person.to_api());
            }
        }
        if lost.passkeys {
            p.passkey_count += 1;
            if p.passkeys.len() < SignInPreview::LIMIT {
                p.passkeys.push(person.to_api());
            }
        }
    }
    p
}

/// What `rules` do to the sign-in of `actor`, made `how`.
fn you(rules: &[SignInRule], actor: &Person, how: &SignedIn) -> SignInYou {
    if covering(rules, &actor.addresses()).is_empty() {
        SignInYou::Outside
    } else if actor.owner {
        SignInYou::Owner
    } else if admits(rules, &actor.addresses(), false, how).is_ok() {
        SignInYou::Meets
    } else {
        SignInYou::SignsOut
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(domains: &[&str], passkeys: bool) -> SignInRule {
        SignInRule {
            domains: domains.iter().map(|d| (*d).to_owned()).collect(),
            provider: GOOGLE.to_owned(),
            passkeys,
        }
    }

    fn google(address: &str) -> SignedIn {
        SignedIn::provider(GOOGLE, Some(address))
    }

    #[test]
    fn methods_read_back_as_tokens_keep_them() {
        for how in [
            SignedIn::password(),
            SignedIn::passkey(),
            google("ada@acme.com"),
        ] {
            let read = SignedIn::read(how.method_str(), how.domain.as_deref());
            assert_eq!(read, how);
        }
        assert_eq!(google("Ada@ACME.com").domain.as_deref(), Some("acme.com"));
        // From before fctl kept it.
        assert_eq!(SignedIn::read(None, None), SignedIn::default());
        assert_eq!(SignedIn::read(Some(""), Some("")), SignedIn::default());
    }

    #[test]
    fn domains_are_kept_lowercase_and_bare() {
        for typed in [
            "acme.com",
            " ACME.com ",
            "*@acme.com",
            "@acme.com",
            "acme.com.",
        ] {
            assert_eq!(domain(typed).ok().as_deref(), Some("acme.com"), "{typed}");
        }
        for typed in ["", "acme", "ada@acme.com", "acme..com", "*@", "acme com"] {
            assert!(domain(typed).is_err(), "{typed}");
        }
    }

    #[test]
    fn rules_are_checked_as_they_are_saved() {
        let checked = check_rules(
            &[rule(&["ACME.dev", "*@acme.com", "acme.com"], false)],
            &[GOOGLE],
        );
        assert_eq!(
            checked.ok(),
            Some(vec![rule(&["acme.com", "acme.dev"], false)])
        );
        // A provider the instance hasn't set up.
        let refused = check_rules(&[rule(&["acme.com"], false)], &[]).err();
        assert!(refused.is_some_and(|e| e.message.contains("GOOGLE_CLIENT_ID")));
        // No domains, a domain in two rules.
        assert!(check_rules(&[rule(&[], false)], &[GOOGLE]).is_err());
        assert!(
            check_rules(
                &[rule(&["acme.com"], false), rule(&["ACME.com"], true)],
                &[GOOGLE]
            )
            .is_err()
        );
        // Too many in all.
        let many: Vec<String> = (0..=MAX_DOMAINS).map(|i| format!("d{i}.com")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(check_rules(&[rule(&many, false)], &[GOOGLE]).is_err());
        assert_eq!(check_rules(&[], &[]).ok(), Some(vec![]));
    }

    #[test]
    fn a_rule_covers_every_account_with_an_address_at_its_domains() {
        let rules = [rule(&["acme.com", "acme.dev"], false)];
        assert_eq!(covering(&rules, &["ada@acme.com"]).len(), 1);
        assert_eq!(covering(&rules, &["ada@ACME.dev"]).len(), 1);
        // A connected address counts as much as the primary one.
        assert_eq!(
            covering(&rules, &["bob@gmail.com", "bob@acme.dev"]).len(),
            1
        );
        assert!(covering(&rules, &["cy@contractor.io"]).is_empty());
        // Only the domain itself, not its subdomains or look-alikes.
        assert!(covering(&rules, &["dee@eu.acme.com", "eve@acme.com.evil.io"]).is_empty());
    }

    #[test]
    fn only_the_provider_as_a_covered_address_meets_a_rule() {
        let r = rule(&["acme.com", "acme.dev"], false);
        assert!(meets(&r, &google("ada@acme.com")));
        assert!(meets(&r, &google("ada@acme.dev")));
        // Google, but as a personal account: not the organization's.
        assert!(!meets(&r, &google("ada@gmail.com")));
        // Google, vouching for no address at all.
        assert!(!meets(&r, &SignedIn::provider(GOOGLE, None)));
        // Another provider, a password, a passkey, a sign-in from before.
        assert!(!meets(
            &r,
            &SignedIn::provider("okta", Some("ada@acme.com"))
        ));
        assert!(!meets(&r, &SignedIn::password()));
        assert!(!meets(&r, &SignedIn::passkey()));
        assert!(!meets(&r, &SignedIn::default()));
        // Passkeys, where the rule allows them.
        assert!(meets(&rule(&["acme.com"], true), &SignedIn::passkey()));
    }

    #[test]
    fn covered_accounts_sign_in_only_as_their_rule_says() {
        let rules = [rule(&["acme.com"], false)];
        let ada = ["ada@acme.com"];
        assert!(admits(&rules, &ada, false, &google("ada@acme.com")).is_ok());
        let refused = admits(&rules, &ada, false, &SignedIn::password()).err();
        assert_eq!(
            refused,
            Some(Required {
                provider: GOOGLE.to_owned(),
                domains: vec!["acme.com".to_owned()],
                passkeys: false,
            })
        );
        assert!(admits(&rules, &ada, false, &SignedIn::default()).is_err());
        // Others sign in as they like.
        assert!(admits(&rules, &["cy@contractor.io"], false, &SignedIn::password()).is_ok());
        // The primary instance owner keeps every way in.
        assert!(admits(&rules, &ada, true, &SignedIn::password()).is_ok());
        assert_eq!(required(&rules, &ada, true), None);
        assert!(required(&rules, &ada, false).is_some());
        // An account at two organizations' domains: either one's provider.
        let both = [rule(&["acme.com"], false), rule(&["beta.io"], false)];
        let dee = ["dee@acme.com", "dee@beta.io"];
        assert!(admits(&both, &dee, false, &google("dee@beta.io")).is_ok());
        assert!(admits(&both, &dee, false, &google("dee@gmail.com")).is_err());
        // What it requires names the account's covered domains only.
        let refused = admits(
            &[rule(&["acme.com", "acme.dev"], false)],
            &["bob@gmail.com", "bob@acme.dev"],
            false,
            &SignedIn::password(),
        )
        .err();
        assert_eq!(
            refused.map(|r| r.domains),
            Some(vec!["acme.dev".to_owned()])
        );
    }

    #[test]
    fn requiring_moves_domains_between_rules() {
        let owned = |d: &[&str]| d.iter().map(|d| (*d).to_owned()).collect::<Vec<_>>();
        let rules = vec![rule(&["acme.com"], false)];
        // Into the rule that says the same, as typed.
        assert_eq!(
            with_required(&rules, &owned(&["*@ACME.dev"]), GOOGLE, false),
            vec![rule(&["acme.com", "acme.dev"], false)]
        );
        // Out of it into one that lets passkeys in; the empty one goes.
        assert_eq!(
            with_required(&rules, &owned(&["acme.com", "acme.io"]), GOOGLE, true),
            vec![rule(&["acme.com", "acme.io"], true)]
        );
        // Again, it changes nothing.
        assert_eq!(
            with_required(&rules, &owned(&["acme.com"]), GOOGLE, false),
            rules
        );
        // Letting a domain go, and the last one.
        let two = vec![rule(&["acme.com", "acme.dev"], false)];
        assert_eq!(
            without(&two, &owned(&["acme.dev"])),
            vec![rule(&["acme.com"], false)]
        );
        assert!(without(&two, &owned(&["acme.com", "@acme.dev"])).is_empty());
        assert_eq!(without(&two, &owned(&["beta.io"])), two);
    }

    #[test]
    fn admins_unlink_members_and_only_the_owner_unlinks_admins() {
        use fctl_proto::session::InstanceRole::{Admin, Member, Owner};
        assert!(check_unlink(Admin, Member, false).is_ok());
        assert!(check_unlink(Owner, Member, false).is_ok());
        assert!(check_unlink(Owner, Admin, false).is_ok());
        let refused = |actor, target, own| check_unlink(actor, target, own).err().map(|e| e.code);
        // An admin's, only the owner; the owner's, nobody; one's own, in Settings.
        assert_eq!(
            refused(Admin, Admin, false),
            Some(fctl_proto::ErrorCode::Forbidden)
        );
        assert_eq!(
            refused(Admin, Owner, false),
            Some(fctl_proto::ErrorCode::Forbidden)
        );
        assert_eq!(
            refused(Owner, Owner, true),
            Some(fctl_proto::ErrorCode::Conflict)
        );
        assert_eq!(
            refused(Admin, Admin, true),
            Some(fctl_proto::ErrorCode::Conflict)
        );
    }

    #[test]
    fn domains_read_as_a_list() {
        let words =
            |d: &[&str]| domains_words(&d.iter().map(|d| (*d).to_owned()).collect::<Vec<_>>());
        assert_eq!(words(&["acme.com"]), "acme.com");
        assert_eq!(words(&["acme.com", "acme.dev"]), "acme.com and acme.dev");
        assert_eq!(
            words(&["acme.com", "acme.dev", "acme.io"]),
            "acme.com, acme.dev and acme.io"
        );
    }

    fn person(id: &str, addresses: &[&str]) -> Person {
        Person {
            id: id.to_owned(),
            name: id.to_owned(),
            email: addresses
                .first()
                .map(|a| (*a).to_owned())
                .unwrap_or_default(),
            addresses: addresses.iter().map(|a| (*a).to_owned()).collect(),
            ..Person::default()
        }
    }

    /// Ada owns the instance; Bob and Cy work at Acme, Bob with a password
    /// and a passkey, Cy with Google linked as his Acme address; Dee's
    /// account an admin token made, and nobody signed in to it; Eve, a
    /// contractor, signs in with a password.
    fn people() -> Vec<Person> {
        vec![
            Person {
                owner: true,
                password: true,
                passkeys: true,
                ..person("ada", &["ada@acme.com"])
            },
            Person {
                password: true,
                passkeys: true,
                identities: vec![(GOOGLE.to_owned(), Some("gmail.com".to_owned()))],
                tokens: vec![
                    ("pat".to_owned(), SignedIn::password()),
                    ("cli".to_owned(), SignedIn::default()),
                    ("admin".to_owned(), SignedIn::password()),
                    ("pat".to_owned(), google("bob@acme.com")),
                ],
                ..person("bob", &["bob@acme.com"])
            },
            Person {
                identities: vec![
                    (GOOGLE.to_owned(), Some("acme.com".to_owned())),
                    // A consumer account showing a work address Google
                    // doesn't vouch for.
                    (GOOGLE.to_owned(), None),
                ],
                ..person("cy", &["cy@acme.com"])
            },
            person("dee", &["dee@acme.com"]),
            Person {
                password: true,
                ..person("eve", &["eve@contractor.io"])
            },
        ]
    }

    #[test]
    fn requiring_google_shows_whose_way_in_stops_working() {
        let after = [rule(&["acme.com"], false)];
        let p = preview(&[], &after, &people(), "ada", &SignedIn::password());
        assert_eq!(p.covered, 4);
        let names = |l: &[SignInPerson]| l.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&p.passwords), ["bob"]);
        assert_eq!(names(&p.passkeys), ["bob"]);
        assert_eq!((p.password_count, p.passkey_count), (1, 1));
        // Bob's Google account is his own Gmail, which doesn't count.
        assert_eq!(p.linked, 1);
        // His access token and CLI sign-in from before stop, and his admin
        // token; the one he made after signing in with Google doesn't.
        assert_eq!((p.access_tokens, p.admin_tokens), (2, 1));
        assert_eq!(p.first, 1);
        assert_eq!(p.others, 1);
        assert_eq!(p.released, 0);
        assert_eq!(p.owner.map(|o| o.name), Some("ada".to_owned()));
        assert_eq!(p.you, SignInYou::Owner);
    }

    #[test]
    fn a_change_shows_only_what_it_changes() {
        let before = [rule(&["acme.com"], true)];
        // Passkeys off: Bob's stop; his password had stopped already.
        let after = [rule(&["acme.com"], false)];
        let p = preview(&before, &after, &people(), "ada", &SignedIn::password());
        assert_eq!((p.password_count, p.passkey_count), (0, 1));
        // The rule goes: those it covered are released, the owner aside.
        let p = preview(&before, &[], &people(), "ada", &SignedIn::password());
        assert_eq!(p.covered, 0);
        assert_eq!(p.released, 3);
        assert_eq!(p.others, 5);
        assert_eq!(p.you, SignInYou::Outside);
    }

    #[test]
    fn an_admin_never_signs_themselves_out() {
        let after = [rule(&["acme.com"], false)];
        let people = people();
        // Bob signed in with his password: requiring Google would end it.
        let p = preview(&[], &after, &people, "bob", &SignedIn::password());
        assert_eq!(p.you, SignInYou::SignsOut);
        // With Google as his Gmail account too.
        let p = preview(&[], &after, &people, "bob", &google("bob@gmail.com"));
        assert_eq!(p.you, SignInYou::SignsOut);
        // As his Acme address, it meets it.
        let p = preview(&[], &after, &people, "bob", &google("bob@acme.com"));
        assert_eq!(p.you, SignInYou::Meets);
        // Eve isn't at Acme.
        let p = preview(&[], &after, &people, "eve", &SignedIn::password());
        assert_eq!(p.you, SignInYou::Outside);
    }

    #[test]
    fn whom_a_change_takes_a_way_in_from() {
        let after = [rule(&["acme.com"], false)];
        let people = people();
        let lost: Vec<(&str, Lost)> = affected(&[], &after, &people)
            .into_iter()
            .map(|(p, l)| (p.id.as_str(), l))
            .collect();
        assert_eq!(
            lost,
            [(
                "bob",
                Lost {
                    password: true,
                    passkeys: true
                }
            )]
        );
        // Nothing to take where nothing changes, or a rule goes.
        assert!(affected(&after, &after, &people).is_empty());
        assert!(affected(&after, &[], &people).is_empty());
    }

    #[test]
    fn coverage_counts_who_moved_and_who_still_has_a_password() {
        let c = coverage(&rule(&["acme.com"], false), &people());
        assert_eq!(c.people, 4);
        assert_eq!(c.linked, 1);
        // Bob's; Ada's stays, as the owner's.
        assert_eq!(c.passwords, 1);
    }
}
