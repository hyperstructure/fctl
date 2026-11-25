//! `fctl admin sign-in …`: how people sign in to the instance, and the
//! domains whose people sign in with a provider only (instance admins).

use crate::style;
use anyhow::{Result, bail};
use clap::Subcommand;
use fctl_client::Api;
use fctl_domain::enrollment::parse_minutes;
use fctl_domain::session_policy::describe as describe_policy;
use fctl_domain::sign_in::{GOOGLE, domains_words, with_required, without};
use fctl_proto::api::{
    SessionPolicy, SessionPolicyPreview, SignInOverview, SignInPerson, SignInPreview, SignInRule,
    SignInYou, UpdateInstanceSettingsRequest,
};
use std::io::{BufRead, IsTerminal, Write};

#[derive(Subcommand)]
pub enum SignInCmd {
    /// Show how people sign in (the default): the providers, the domains
    /// whose people sign in with one only, and who has which way in.
    Show,
    /// Require a provider of the people at some domains, as an address
    /// there: their passwords, other providers and sign-ins made before
    /// stop working, and their passkeys alone too unless --passkeys. It
    /// shows whom that concerns and asks first, unless --yes. The primary
    /// instance owner keeps every way in, for when the provider is down.
    Require {
        /// The domains, like acme.com.
        #[arg(required = true, value_name = "DOMAIN")]
        domains: Vec<String>,
        /// The provider they sign in with.
        #[arg(long, default_value = GOOGLE)]
        provider: String,
        /// Let a passkey alone sign them in too: quick, but past the
        /// provider's checks (its two-step verification, device checks,
        /// offboarding).
        #[arg(long)]
        passkeys: bool,
        /// Save it without asking.
        #[arg(long)]
        yes: bool,
    },
    /// Stop requiring a provider of the people at some domains: they sign
    /// in as they like again, those who still have a password with it.
    /// Takes confirming it's you, where the instance asks for that.
    Unrequire {
        /// The domains, like acme.com.
        #[arg(required = true, value_name = "DOMAIN")]
        domains: Vec<String>,
        /// Save it without asking.
        #[arg(long)]
        yes: bool,
    },
    /// Show or change how long sign-ins last and who makes access tokens.
    /// A change shows what it ends at once (browsers and CLIs signed in
    /// longer than it lets them, access tokens older) and asks first,
    /// unless --yes. Letting them last longer takes confirming it's you,
    /// where the instance asks for that.
    Policies {
        /// How long a browser stays signed in after its sign-in, however
        /// much it's used, in whole hours (12h, 1d, 7d), or `none`: as
        /// long as it's used.
        #[arg(long, value_name = "DURATION|none")]
        browser_max: Option<String>,
        /// How long a browser may go unused before it's signed out: 5m to
        /// 30d (the default).
        #[arg(long, value_name = "DURATION")]
        browser_idle: Option<String>,
        /// How long a CLI stays signed in after `fctl login`, in whole days
        /// (30d), or `none`: as long as it's used, until 180 days unused.
        #[arg(long, value_name = "DURATION|none")]
        cli_max: Option<String>,
        /// The longest an access token may last, in whole days (90d), or
        /// `none`: as long as its maker chooses. Older ones end.
        #[arg(long, value_name = "DURATION|none")]
        token_max: Option<String>,
        /// Whether members make access tokens. Off, only admins do, and
        /// members' stop working: they use the CLI signed in as
        /// themselves.
        #[arg(long, value_name = "ON|OFF", value_parser = clap::builder::BoolishValueParser::new(), hide_possible_values = true)]
        member_tokens: Option<bool>,
        /// Save it without asking.
        #[arg(long)]
        yes: bool,
    },
}

pub async fn run(api: &Api, cmd: Option<SignInCmd>, json: bool) -> Result<()> {
    match cmd.unwrap_or(SignInCmd::Show) {
        SignInCmd::Show => {
            let overview = api.sign_in_overview().await?;
            if json {
                return style::print_json(&overview);
            }
            print_overview(&overview);
        }
        SignInCmd::Require {
            domains,
            provider,
            passkeys,
            yes,
        } => {
            let overview = api.sign_in_overview().await?;
            let next = with_required(&overview.rules, &domains, &provider, passkeys);
            change(api, &overview, next, yes, json).await?;
        }
        SignInCmd::Unrequire { domains, yes } => {
            let overview = api.sign_in_overview().await?;
            let next = without(&overview.rules, &domains);
            if next == overview.rules {
                bail!(
                    "No rule names {}: `fctl admin sign-in` lists them.",
                    domains.join(", ")
                );
            }
            change(api, &overview, next, yes, json).await?;
        }
        SignInCmd::Policies {
            browser_max,
            browser_idle,
            cli_max,
            token_max,
            member_tokens,
            yes,
        } => {
            let overview = api.sign_in_overview().await?;
            let was = overview.sessions.clone();
            let mut next = was.clone();
            if let Some(d) = browser_max {
                next.browser_max_hours = whole(&d, 60, "hours", "12h or 1d")?;
            }
            if let Some(d) = browser_idle {
                next.browser_idle_minutes = parse_minutes(&d)
                    .map_err(anyhow::Error::msg)?
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Browsers are signed out after 30 days unused at most: give 5m to 30d"
                        )
                    })?;
            }
            if let Some(d) = cli_max {
                next.cli_max_days = whole(&d, 1440, "days", "30d")?;
            }
            if let Some(d) = token_max {
                next.access_token_max_days = whole(&d, 1440, "days", "90d")?;
            }
            if let Some(on) = member_tokens {
                next.member_access_tokens = on;
            }
            if next == was {
                if json {
                    return style::print_json(&was);
                }
                print_policy(&was);
                return Ok(());
            }
            change_policy(api, next, yes, json).await?;
        }
    }
    Ok(())
}

/// A duration as people type it (`12h`, `7d`), in whole `unit`s of `per`
/// minutes; `none` is 0, no limit.
fn whole(input: &str, per: u32, unit: &str, example: &str) -> Result<u32> {
    match parse_minutes(input).map_err(anyhow::Error::msg)? {
        None => Ok(0),
        Some(m) if m % per == 0 => Ok(m / per),
        Some(_) => bail!("{input:?} isn't whole {unit}: give one like {example}, or none"),
    }
}

/// Shows what saving `next` ends at once, asks unless `yes`, saves it and
/// shows the policy now.
async fn change_policy(api: &Api, next: SessionPolicy, yes: bool, json: bool) -> Result<()> {
    let preview = api.session_policy_preview(&next).await?;
    for line in policy_consequences(&preview) {
        eprintln!("{line}");
    }
    if !yes {
        if !std::io::stdin().is_terminal() {
            bail!("Not saved: pass --yes to save it without being asked");
        }
        eprint!("Save it? [y/N] ");
        std::io::stderr().flush()?;
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        if !matches!(line.trim(), "y" | "Y" | "yes" | "Yes") {
            bail!("Not saved");
        }
    }
    let saved = api
        .update_instance_settings(&UpdateInstanceSettingsRequest {
            browser_max_hours: Some(next.browser_max_hours),
            browser_idle_minutes: Some(next.browser_idle_minutes),
            cli_max_days: Some(next.cli_max_days),
            access_token_max_days: Some(next.access_token_max_days),
            member_access_tokens: Some(next.member_access_tokens),
            ..UpdateInstanceSettingsRequest::default()
        })
        .await?;
    let now = saved.session_policy();
    if json {
        return style::print_json(&now);
    }
    println!("{} Saved", style::accent("✓"));
    print_policy(&now);
    Ok(())
}

/// What saving a session policy ends, in the sentences the web app shows
/// too.
pub fn policy_consequences(p: &SessionPolicyPreview) -> Vec<String> {
    let count = |n: u32, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    let mut signed_out = Vec::new();
    if p.browsers > 0 {
        signed_out.push(count(p.browsers, "browser", "browsers"));
    }
    if p.clis > 0 {
        signed_out.push(count(p.clis, "CLI", "CLIs"));
    }
    let mut lines = Vec::new();
    if !signed_out.is_empty() {
        lines.push(format!(
            "Saving signs out {} at once{}: their people sign in again.",
            domains_words(&signed_out),
            if p.you { ", yours among them" } else { "" }
        ));
    } else if p.you {
        lines.push("Saving signs you out at once: you sign in again.".to_owned());
    }
    if p.access_tokens > 0 {
        let members = match p.member_tokens {
            0 => String::new(),
            n if n == p.access_tokens => ", all of them members'".to_owned(),
            n => format!(", {n} of them members'"),
        };
        lines.push(format!(
            "It ends {}{members}: scripts and CI that use {} need new ones.",
            count(p.access_tokens, "access token", "access tokens"),
            if p.access_tokens == 1 { "it" } else { "them" }
        ));
    }
    if lines.is_empty() {
        lines.push("Nobody is signed out now: sign-ins end as it says from here on.".to_owned());
    }
    lines
}

fn print_policy(p: &SessionPolicy) {
    for line in describe_policy(p) {
        println!("{line}");
    }
}

/// Shows what saving `next` would do, asks unless `yes`, saves it and
/// shows how people sign in now. What it shows before it asks goes to
/// stderr, so that `--json` leaves stdout to the result.
async fn change(
    api: &Api,
    overview: &SignInOverview,
    next: Vec<SignInRule>,
    yes: bool,
    json: bool,
) -> Result<()> {
    let preview = api.sign_in_preview(&next).await?;
    let name = provider_name(overview, &next);
    for line in consequences(&preview, &name) {
        eprintln!("{line}");
    }
    if preview.you == SignInYou::SignsOut {
        bail!(
            "Not saved: it would sign you out, as you signed in another way. Sign in with {name} as an address at one of its domains first (fctl login, then continue with {name}), which shows that it works."
        );
    }
    if !yes {
        if !std::io::stdin().is_terminal() {
            bail!("Not saved: pass --yes to save it without being asked");
        }
        eprint!("Save it? [y/N] ");
        std::io::stderr().flush()?;
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        if !matches!(line.trim(), "y" | "Y" | "yes" | "Yes") {
            bail!("Not saved");
        }
    }
    api.update_instance_settings(&UpdateInstanceSettingsRequest {
        sign_in: Some(next),
        ..UpdateInstanceSettingsRequest::default()
    })
    .await?;
    let now = api.sign_in_overview().await?;
    if json {
        return style::print_json(&now);
    }
    println!("{} Saved", style::accent("✓"));
    print_overview(&now);
    Ok(())
}

/// The name of the provider `rules` require, or of the instance's first.
fn provider_name(overview: &SignInOverview, rules: &[SignInRule]) -> String {
    let id = rules.first().map(|r| r.provider.as_str()).unwrap_or(GOOGLE);
    overview
        .providers
        .iter()
        .find(|p| p.id == id)
        .map_or_else(|| id.to_owned(), |p| p.name.clone())
}

fn people(n: u32) -> String {
    if n == 1 {
        "1 person".to_owned()
    } else {
        format!("{n} people")
    }
}

/// Up to five names, and how many more.
fn names(list: &[SignInPerson], all: u32) -> String {
    let shown: Vec<&str> = list.iter().take(5).map(|p| p.name.as_str()).collect();
    let more = all as usize - shown.len().min(all as usize);
    if more == 0 {
        shown.join(", ")
    } else {
        format!("{} and {more} more", shown.join(", "))
    }
}

/// What a change does, in the sentences the web app shows too.
pub fn consequences(p: &SignInPreview, provider: &str) -> Vec<String> {
    let mut lines = Vec::new();
    if p.password_count > 0 {
        lines.push(format!(
            "{} {} with a password today: their next sign-in goes through {provider}, and their {} stop working ({}).",
            people(p.password_count),
            if p.password_count == 1 { "signs in" } else { "sign in" },
            if p.password_count == 1 { "password" } else { "passwords" },
            names(&p.passwords, p.password_count)
        ));
    }
    if p.passkey_count > 0 {
        lines.push(format!(
            "{} {} passkeys that stop signing them in alone; they still confirm it's them after {provider} ({}).",
            people(p.passkey_count),
            if p.passkey_count == 1 { "has" } else { "have" },
            names(&p.passkeys, p.passkey_count)
        ));
    }
    if p.access_tokens > 0 {
        lines.push(format!(
            "Their {} made with another way in {}: they make new ones after signing in with {provider}.",
            if p.access_tokens == 1 {
                "1 access token or CLI sign-in".to_owned()
            } else {
                format!("{} access tokens and CLI sign-ins", p.access_tokens)
            },
            if p.access_tokens == 1 { "goes" } else { "go" }
        ));
    }
    if p.admin_tokens > 0 {
        lines.push(format!(
            "{} made with another way in {}: a program that provisions people with one needs a new one.",
            if p.admin_tokens == 1 {
                "1 admin token".to_owned()
            } else {
                format!("{} admin tokens", p.admin_tokens)
            },
            if p.admin_tokens == 1 { "stops working" } else { "stop working" }
        ));
    }
    if p.linked > 0 {
        lines.push(format!(
            "{} {} with {provider} already.",
            people(p.linked),
            if p.linked == 1 { "signs in" } else { "sign in" }
        ));
    }
    if p.first > 0 {
        lines.push(format!(
            "{} {} no way in yet: their first sign-in with {provider} links it.",
            people(p.first),
            if p.first == 1 { "has" } else { "have" },
        ));
    }
    if p.released > 0 {
        lines.push(format!(
            "{} {} as they like again; those who still have a password, with it.",
            people(p.released),
            if p.released == 1 {
                "signs in"
            } else {
                "sign in"
            }
        ));
    }
    // Nothing anyone signs in with stops or starts working: the rest is context.
    if p.password_count == 0
        && p.passkey_count == 0
        && p.access_tokens == 0
        && p.admin_tokens == 0
        && p.released == 0
    {
        lines.insert(0, "Nobody's way in changes.".to_owned());
        return lines;
    }
    if p.covered > 0 && p.others > 0 {
        lines.push(format!(
            "{} at other domains {} as before.",
            people(p.others),
            if p.others == 1 { "signs in" } else { "sign in" }
        ));
    }
    match (p.you, &p.owner) {
        (SignInYou::Owner, _) => lines.push(format!(
            "You keep your password and passkeys, as the primary instance owner: the way in if {provider} is down."
        )),
        (_, Some(owner)) => lines.push(format!(
            "{}, the primary instance owner, keeps their password and passkeys: the way in if {provider} is down.",
            owner.name
        )),
        _ => {}
    }
    if p.email && (p.password_count > 0 || p.passkey_count > 0) {
        lines.push("fctl emails each of them what changes.".to_owned());
    }
    lines
}

fn print_overview(o: &SignInOverview) {
    for p in &o.providers {
        println!(
            "{:<12} {}",
            style::bold(&p.name),
            if p.configured {
                style::accent(&match p.people {
                    0 => "set up · nobody has linked it yet".to_owned(),
                    n => format!("set up · {} linked", people(n)),
                })
            } else {
                style::dim(&format!("not set up: the deployment sets {}", p.setup))
            }
        );
        if p.configured {
            println!(
                "  {}",
                style::dim(&format!("redirect URI {}", p.redirect_url))
            );
        }
    }
    if o.rules.is_empty() {
        println!(
            "{}",
            style::dim("No domain requires a provider: people sign in as they like.")
        );
    }
    for (rule, c) in o.rules.iter().zip(&o.coverage) {
        let name = o
            .providers
            .iter()
            .find(|p| p.id == rule.provider)
            .map_or(rule.provider.as_str(), |p| p.name.as_str());
        println!(
            "{} {}",
            style::bold(&domains_words(&rule.domains)),
            style::accent(&format!("{name} only"))
        );
        let mut about = vec![format!("{}, {} with {name}", people(c.people), c.linked)];
        if c.passwords > 0 {
            about.push(format!(
                "{} still {} a password, which goes at their next sign-in",
                c.passwords,
                if c.passwords == 1 { "has" } else { "have" },
            ));
        }
        about.push(if rule.passkeys {
            "passkeys alone sign them in too".to_owned()
        } else {
            "passkeys confirm it's them, and don't sign them in alone".to_owned()
        });
        println!("  {}", style::dim(&about.join("; ")));
    }
    println!(
        "Accounts     {}",
        if o.self_sign_up {
            style::accent("made by signing in")
        } else {
            style::dim("only from invitations and admin tokens")
        }
    );
    println!(
        "  {}",
        style::dim(&format!(
            "{} active: {} with a password, {} with a passkey, {} with no way in yet",
            people(o.active),
            o.passwords,
            o.passkeys,
            o.no_way_in
        ))
    );
    println!("{}", style::bold("Staying signed in"));
    for line in describe_policy(&o.sessions) {
        println!("  {}", style::dim(&line));
    }
    if let Some(owner) = &o.owner
        && !o.rules.is_empty()
    {
        println!(
            "  {}",
            style::dim(&format!(
                "{}, the primary instance owner, keeps their password and passkeys whatever the rules say.",
                owner.name
            ))
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::UserId;

    fn person(name: &str) -> SignInPerson {
        SignInPerson {
            id: UserId(name.to_lowercase()),
            name: name.to_owned(),
            email: format!("{}@acme.com", name.to_lowercase()),
            avatar_url: None,
        }
    }

    fn preview() -> SignInPreview {
        SignInPreview {
            covered: 42,
            passwords: vec![person("Bob"), person("Cy")],
            password_count: 38,
            passkeys: vec![person("Bob")],
            passkey_count: 1,
            access_tokens: 5,
            admin_tokens: 1,
            linked: 3,
            first: 0,
            released: 0,
            others: 4,
            owner: Some(person("Ada")),
            you: SignInYou::Owner,
            email: true,
        }
    }

    #[test]
    fn requiring_a_provider_says_whom_it_concerns() {
        let lines = consequences(&preview(), "Google");
        assert_eq!(
            lines[0],
            "38 people sign in with a password today: their next sign-in goes through Google, and their passwords stop working (Bob, Cy and 36 more)."
        );
        assert!(lines.contains(&"4 people at other domains sign in as before.".to_owned()));
        assert!(lines.contains(&"Their 5 access tokens and CLI sign-ins made with another way in go: they make new ones after signing in with Google.".to_owned()));
        assert!(lines.contains(&"1 admin token made with another way in stops working: a program that provisions people with one needs a new one.".to_owned()));
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("You keep your password"))
        );
        assert_eq!(
            lines.last().map(String::as_str),
            Some("fctl emails each of them what changes.")
        );
        // Nothing to say.
        let quiet = SignInPreview {
            covered: 0,
            passwords: vec![],
            password_count: 0,
            passkeys: vec![],
            passkey_count: 0,
            access_tokens: 0,
            admin_tokens: 0,
            linked: 0,
            first: 0,
            owner: None,
            you: SignInYou::Outside,
            ..preview()
        };
        assert_eq!(consequences(&quiet, "Google"), ["Nobody's way in changes."]);
    }

    #[test]
    fn a_session_policy_says_what_it_ends() {
        let p = SessionPolicyPreview {
            browsers: 14,
            clis: 1,
            access_tokens: 5,
            member_tokens: 3,
            you: true,
        };
        assert_eq!(
            policy_consequences(&p),
            [
                "Saving signs out 14 browsers and 1 CLI at once, yours among them: their people sign in again.",
                "It ends 5 access tokens, 3 of them members': scripts and CI that use them need new ones.",
            ]
        );
        assert_eq!(
            policy_consequences(&SessionPolicyPreview::default()),
            ["Nobody is signed out now: sign-ins end as it says from here on."]
        );
    }

    #[test]
    fn policy_durations_read_in_whole_units() {
        assert_eq!(whole("12h", 60, "hours", "12h").ok(), Some(12));
        assert_eq!(whole("1d", 60, "hours", "12h").ok(), Some(24));
        assert_eq!(whole("none", 60, "hours", "12h").ok(), Some(0));
        assert_eq!(whole("90d", 1440, "days", "90d").ok(), Some(90));
        assert!(whole("36h", 1440, "days", "30d").is_err());
        assert!(whole("90m", 60, "hours", "12h").is_err());
    }
}
