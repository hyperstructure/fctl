//! `fctl account passkeys …`: the passkeys you sign in with. Browsers make
//! them, with a fingerprint, face or screen lock, so adding one happens in
//! Settings.

use crate::style;
use anyhow::{Result, bail};
use clap::Subcommand;
use fctl_client::Api;
use fctl_proto::api::Passkey;

#[derive(Subcommand)]
pub enum PasskeysCmd {
    /// List your passkeys.
    List,
    /// Rename a passkey.
    Rename {
        /// The passkey's id or name.
        passkey: String,
        /// Its new name.
        name: String,
    },
    /// Remove a passkey: it no longer signs you in. Remove it from your
    /// passkey provider too.
    Remove {
        /// The passkey's id or name.
        passkey: String,
    },
}

pub async fn run(api: &Api, cmd: PasskeysCmd, json: bool) -> Result<()> {
    let me = api.me().await?;
    match cmd {
        PasskeysCmd::List => {
            if json {
                return style::print_json(&me.passkeys);
            }
            if me.passkeys.is_empty() {
                println!("No passkeys yet.");
            }
            for p in &me.passkeys {
                println!("{}", line(p));
            }
            println!(
                "{}",
                style::dim(&format!(
                    "Add passkeys in your browser, with your fingerprint, face or screen lock: {}/settings",
                    api.base()
                ))
            );
        }
        PasskeysCmd::Rename { passkey, name } => {
            let p = find(&me.passkeys, &passkey)?;
            let me = api.rename_passkey(p.id.as_str(), &name).await?;
            if json {
                return style::print_json(&me.passkeys);
            }
            println!(
                "{} Renamed it {}",
                style::accent("✓"),
                style::bold(name.trim())
            );
        }
        PasskeysCmd::Remove { passkey } => {
            let p = find(&me.passkeys, &passkey)?;
            let me = api.remove_passkey(p.id.as_str()).await?;
            if json {
                return style::print_json(&me.passkeys);
            }
            println!(
                "{} Removed {}: it no longer signs you in.",
                style::accent("✓"),
                style::bold(&p.name)
            );
            println!(
                "{}",
                style::dim(
                    "Remove it from your passkey provider too, so that it stops offering it."
                )
            );
        }
    }
    Ok(())
}

fn line(p: &Passkey) -> String {
    let used = p.last_used_at.map_or_else(
        || "never used".to_owned(),
        |t| format!("used {}", style::ago(t)),
    );
    let kind = if p.synced { "synced" } else { "this device" };
    format!(
        "{:<28} {}",
        style::bold(&p.name),
        style::dim(&format!(
            "{kind} · added {} · {used} · {}",
            style::ago(p.created_at),
            p.id
        ))
    )
}

/// A passkey by its id, or by a name only one has.
fn find<'a>(passkeys: &'a [Passkey], which: &str) -> Result<&'a Passkey> {
    if let Some(p) = passkeys.iter().find(|p| p.id.as_str() == which) {
        return Ok(p);
    }
    let named: Vec<&Passkey> = passkeys
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case(which))
        .collect();
    match named.as_slice() {
        [p] => Ok(p),
        [] => bail!("You have no passkey {which}. `fctl account passkeys list` lists them."),
        _ => bail!(
            "{} passkeys are called {which}; use one of their ids: {}",
            named.len(),
            named
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
