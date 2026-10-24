//! `fctl projects secrets …`: a project's secrets, which its sandboxes get
//! as environment variables. Nobody reads their values back, so a value
//! goes in through stdin or a prompt that hides it, never as an argument,
//! which shell history would keep.

use crate::style;
use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use fctl_client::Api;
use fctl_proto::api::SecretList;
use std::io::{IsTerminal, Read};

#[derive(Args)]
pub struct SecretsArgs {
    /// The project (slug or id); default from `fctl projects use`.
    #[arg(short, long, global = true)]
    pub project: Option<String>,
    #[command(subcommand)]
    pub cmd: Option<SecretsCmd>,
}

#[derive(Subcommand)]
pub enum SecretsCmd {
    /// Their names, and who set each when (the default).
    #[command(visible_alias = "ls")]
    List,
    /// Set a secret, or replace its value. fctl asks for the value without
    /// showing what you type, or reads it from stdin, without the line
    /// break that ends it.
    #[command(
        after_help = "Examples:\n  fctl projects secrets set STRIPE_KEY -p my-app     # asks for the value\n  \
                      printenv STRIPE_KEY | fctl projects secrets set STRIPE_KEY -p my-app\n  \
                      fctl projects secrets set DEPLOY_KEY -p my-app < deploy_key.pem"
    )]
    Set {
        /// The variable sandboxes get it as: uppercase letters, digits and
        /// underscores, like STRIPE_KEY.
        name: String,
    },
    /// Remove a secret.
    #[command(visible_alias = "rm")]
    Remove {
        /// Its name.
        name: String,
    },
}

pub async fn run(api: &Api, args: SecretsArgs, json: bool) -> Result<()> {
    let project = super::sessions::project_or_default(args.project)?;
    match args.cmd.unwrap_or(SecretsCmd::List) {
        SecretsCmd::List => {
            let list = api.secrets(&project).await?;
            if json {
                return style::print_json(&list);
            }
            if list.secrets.is_empty() {
                println!(
                    "{project} has no secrets. Set one with `fctl projects secrets set NAME -p {project}`."
                );
            }
            print(&list);
        }
        SecretsCmd::Set { name } => {
            // As the web app takes it.
            let name = fctl_domain::validate::env_name("name", &name.to_ascii_uppercase())
                .map_err(|e| anyhow!(e.message))?;
            // Only those who may set it are asked for the value.
            let had = api
                .secrets(&project)
                .await?
                .secrets
                .iter()
                .any(|s| s.name == name);
            let value = value(&name)?;
            let list = api.set_secret(&project, &name, &value).await?;
            if json {
                return style::print_json(&list);
            }
            println!(
                "{} {} {} in {project}",
                style::accent("✓"),
                if had { "Replaced" } else { "Saved" },
                style::bold(&name)
            );
            println!(
                "{}",
                style::dim(
                    "Sessions get it as an environment variable when they next start or resume."
                )
            );
        }
        SecretsCmd::Remove { name } => {
            let list = api.secrets(&project).await?;
            let Some(secret) = list
                .secrets
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(name.trim()))
            else {
                bail!(
                    "{project} has no secret {}. `fctl projects secrets -p {project}` lists them.",
                    name.trim()
                );
            };
            let list = api.delete_secret(&project, &secret.name).await?;
            if json {
                return style::print_json(&list);
            }
            println!(
                "{} Removed {} from {project}",
                style::accent("✓"),
                style::bold(&secret.name)
            );
            println!(
                "{}",
                style::dim("Sessions that have it keep it until they next start or resume.")
            );
        }
    }
    Ok(())
}

fn print(list: &SecretList) {
    for s in &list.secrets {
        let by = s
            .updated_by
            .as_ref()
            .map(|u| format!(" by {}", u.name))
            .unwrap_or_default();
        println!(
            "{:<32} {}",
            style::bold(&s.name),
            style::dim(&format!("updated {}{by}", style::ago(s.updated_at)))
        );
    }
}

/// A secret's value: asked for in a terminal, without showing it, else
/// read from stdin.
fn value(name: &str) -> Result<String> {
    let value = if std::io::stdin().is_terminal() {
        crate::prompt::read_secret(&format!("Value of {name} (input hidden): "))?
    } else {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("reading the value from stdin")?;
        piped(text)
    };
    if value.is_empty() {
        bail!("No value given for {name}: type it when asked, or pipe it in");
    }
    Ok(value)
}

/// What was piped in, without the line breaks that end it (`echo` adds
/// one): the rest is the value, as it came.
fn piped(mut text: String) -> String {
    let end = text.trim_end_matches(['\r', '\n']).len();
    text.truncate(end);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piped_values_lose_only_the_line_breaks_that_end_them() {
        assert_eq!(piped("sk_live_123\n".into()), "sk_live_123");
        assert_eq!(piped("sk_live_123\r\n".into()), "sk_live_123");
        assert_eq!(piped("sk_live_123".into()), "sk_live_123");
        assert_eq!(piped("  spaced  \n\n".into()), "  spaced  ");
        let pem = "-----BEGIN KEY-----\nabc\n-----END KEY-----\n";
        assert_eq!(piped(pem.into()), pem.trim_end());
        assert_eq!(piped("\n".into()), "");
    }
}
