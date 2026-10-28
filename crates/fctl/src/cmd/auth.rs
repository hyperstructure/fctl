//! `fctl login`, `logout`, `whoami`.

use crate::config::{self, CliConfig, Credentials};
use crate::style;
use anyhow::{Result, bail};
use fctl_client::Api;
use fctl_proto::api::DevicePollResponse;
use std::time::Duration;

pub fn open_browser(url: &str) -> bool {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Opens a `claude://` link in the Claude desktop app. False when no app
/// takes such links: macOS `open` fails then, but `xdg-open` may hand the
/// link to a browser, so on Linux it asks `xdg-mime` for a handler first.
pub fn open_claude_app(link: &str) -> bool {
    let handled = cfg!(target_os = "macos")
        || std::process::Command::new("xdg-mime")
            .args(["query", "default", "x-scheme-handler/claude"])
            .stderr(std::process::Stdio::null())
            .output()
            .is_ok_and(|o| o.status.success() && !o.stdout.trim_ascii().is_empty());
    handled && open_browser(link)
}

pub async fn login(url: Option<&str>, no_browser: bool, json: bool) -> Result<()> {
    let server = match url {
        Some(u) => config::normalize_server(u),
        None => config::server(None)?,
    };
    let api = Api::new(&server, None);
    let info = api.instance().await?;
    if info.setup_required {
        bail!(
            "{} has no owner yet. Open the setup link from its deployment first.",
            server
        );
    }
    let client = format!("fctl on {}", fctl_host::config::hostname());
    let start = api.device_start(&client).await?;
    // The code is approved where the server sends people: on another site,
    // it would sign this machine's user in there, and give that site the
    // token, not this fctl.
    if !fctl_client::http::same_site(&server, &start.verification_uri_complete) {
        match fctl_client::http::origin(&start.verification_uri_complete) {
            Some(elsewhere) => bail!(
                "{server} asks you to approve this sign-in on another site ({elsewhere}), which would sign you in there for it. To sign in to {elsewhere}, run `fctl login {elsewhere}`."
            ),
            None => bail!("{server} sent a sign-in link that isn't a web address"),
        }
    }
    if !json {
        println!("{}", style::bold(&format!("Sign in to {}", info.name)));
        println!();
        println!("  Code: {}", style::accent(&start.user_code));
        println!("  Link: {}", start.verification_uri_complete);
        println!();
    }
    if !no_browser && open_browser(&start.verification_uri_complete) && !json {
        println!(
            "{}",
            style::dim("Opened your browser. Confirm the code there.")
        );
    }
    let mut interval = Duration::from_secs(start.interval.max(1) as u64);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(start.expires_in as u64);
    loop {
        tokio::time::sleep(interval).await;
        if tokio::time::Instant::now() > deadline {
            bail!("The code expired. Run `fctl login` again.");
        }
        match api.device_poll(&start.device_code).await? {
            DevicePollResponse::Pending => {}
            DevicePollResponse::SlowDown => interval += Duration::from_secs(1),
            DevicePollResponse::Denied => bail!("The sign-in was denied."),
            DevicePollResponse::Expired | DevicePollResponse::Verified { .. } => {
                bail!("The code expired. Run `fctl login` again.")
            }
            DevicePollResponse::Approved { token, me } => {
                let mut creds = Credentials::load();
                creds.tokens.insert(server.clone(), token);
                creds.save()?;
                let mut cfg = CliConfig::load();
                cfg.server = Some(server.clone());
                cfg.save()?;
                if json {
                    return style::print_json(&*me);
                }
                println!(
                    "{} Signed in as {} <{}>",
                    style::accent("✓"),
                    me.user.name,
                    me.user.email
                );
                if me.claude.is_empty() {
                    println!(
                        "{}",
                        style::dim(
                            "Next: connect your Claude subscription with `fctl claude connect`."
                        )
                    );
                }
                return Ok(());
            }
        }
    }
}

pub async fn logout(server: Option<&str>) -> Result<()> {
    let server = config::server(server)?;
    if let Some(token) = config::token(&server) {
        let _ = Api::new(&server, Some(token)).logout().await;
    }
    let mut creds = Credentials::load();
    creds.tokens.remove(&server);
    creds.save()?;
    println!("Signed out of {server}");
    Ok(())
}

/// Your account, for commands about it: a limited access token sees only
/// whose it is, which would read as an account without addresses, Claude
/// connections or passkeys.
pub async fn account(api: &Api) -> Result<fctl_proto::api::Me> {
    let me = api.me().await?;
    if me.token.as_ref().is_some_and(|t| t.scope.is_some()) {
        anyhow::bail!(
            "This access token is limited to projects and sessions, so it doesn't see your account. Sign in (`fctl login`), or use a token without limits."
        );
    }
    Ok(me)
}

pub async fn whoami(server: Option<&str>, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let me = api.me().await?;
    if json {
        return style::print_json(&me);
    }
    println!("{} <{}>", style::bold(&me.user.name), me.user.email);
    println!(
        "{}",
        style::dim(&format!(
            "{} · {} · {}",
            api.base(),
            me.user.role.as_str(),
            me.user.id
        ))
    );
    // A limited token sees only who it belongs to and its workspaces.
    let limited = me.token.as_ref().is_some_and(|t| t.scope.is_some());
    if let Some(t) = &me.token {
        let what = t.scope.as_ref().map_or_else(
            || "Does everything you can.".to_owned(),
            super::tokens::describe,
        );
        println!(
            "Access token {}: {}",
            style::bold(&style::terminal_safe(
                t.name.as_deref().unwrap_or(t.id.as_str())
            )),
            style::terminal_safe(&what)
        );
    }
    let others: Vec<&str> = me
        .emails
        .iter()
        .filter(|e| !e.primary)
        .map(|e| e.email.as_str())
        .collect();
    if !others.is_empty() {
        println!("Also signs in as {}", others.join(", "));
    }
    println!("{}", super::account::agents_call_you(&me.user));
    if let Some(email) = &me.commit_email {
        println!("Commits credit you as {email}");
    }
    if let Ok(list) = api.workspaces().await {
        if list.workspaces.is_empty() {
            println!(
                "{}",
                style::amber("No workspace yet: create one with `fctl workspaces create <name>`.")
            );
        } else {
            println!(
                "Workspaces: {}",
                list.workspaces
                    .iter()
                    .map(|w| format!("{} ({})", w.name, super::workspaces::role(w.role)))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !list.invitations.is_empty() {
            println!(
                "{}",
                style::accent(&format!(
                    "Invited to {}: join with `fctl workspaces join`",
                    list.invitations
                        .iter()
                        .map(|i| i.workspace.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            );
        }
    }
    if limited {
        return Ok(());
    }
    if !me.passkeys.is_empty() {
        println!(
            "Passkeys: {}",
            me.passkeys
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let two_factor = super::two_factor::summary(&me.two_factor);
    println!(
        "Two-factor sign-in: {}",
        if me.two_factor.setup_required {
            style::amber(&two_factor)
        } else {
            two_factor
        }
    );
    if me.claude.is_empty() {
        println!(
            "{}",
            style::amber("No Claude subscription connected: run `fctl claude connect`.")
        );
    }
    for c in &me.claude {
        let rc = if c.remote_control {
            "Remote Control"
        } else {
            "inference only"
        };
        println!(
            "Claude: {} ({rc}){}",
            c.account_email
                .as_deref()
                .or(c.label.as_deref())
                .unwrap_or(c.kind.as_str()),
            if c.is_default { " · default" } else { "" }
        );
    }
    Ok(())
}
