//! `fctl host claude …`: keep your Claude token on this runner, so that
//! fctl never has it (docs/architecture.md#claude-credentials).

use crate::style;
use anyhow::{Context, Result, bail};
use clap::Subcommand;
use fctl_domain::claude::masked;
use fctl_host::claude_token::{self, Kept};
use fctl_host::{HostConfig, service};
use fctl_proto::runner::RunnerClaudeToken;
use std::io::{BufRead, IsTerminal};
use std::path::Path;

#[derive(Subcommand)]
pub enum HostClaudeCmd {
    /// Keep a token from `claude setup-token` on this runner, for your
    /// sessions on the Claude connection you keep on your runners (asked
    /// for, or read from stdin). It replaces the one kept here.
    SetToken,
    /// Whether this runner keeps your Claude token, and whether fctl knows.
    Status,
    /// Remove the Claude token from this runner. Sessions running here keep
    /// it until they stop.
    RemoveToken,
}

pub async fn run(cmd: HostClaudeCmd, json: bool) -> Result<()> {
    // A machine that isn't a runner yet keeps it where its host will look.
    let cfg = HostConfig::load().unwrap_or_default();
    let path = cfg.claude_token();
    let name = runner_name(&cfg);
    match cmd {
        HostClaudeCmd::SetToken => set_token(&cfg, &path, &name, json),
        HostClaudeCmd::Status => status(&path, &name, json).await,
        HostClaudeCmd::RemoveToken => remove_token(&path, &name, json),
    }
}

/// What to call this machine: its runner's name, else "this machine".
fn runner_name(cfg: &HostConfig) -> String {
    if HostConfig::path().exists() {
        cfg.name.clone()
    } else {
        "This machine".into()
    }
}

fn set_token(cfg: &HostConfig, path: &Path, name: &str, json: bool) -> Result<()> {
    let before = claude_token::load(path).ok().flatten();
    let token = if std::io::stdin().is_terminal() {
        println!(
            "Your sessions on the Claude connection you keep on your runners run with the token"
        );
        println!(
            "this runner keeps. fctl never receives it: it stays on this machine and goes only"
        );
        println!("into your sessions' sandboxes here.");
        println!();
        println!("Create one with Claude Code, here or on any machine you use it on:");
        println!();
        println!("    {}", style::accent("claude setup-token"));
        println!();
        crate::prompt::read_secret("Paste the token (input hidden): ")?
    } else {
        // `fctl host claude set-token < file`, or from a secret manager.
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        line.trim().to_owned()
    };
    if token.is_empty() {
        bail!("No token given: paste the one `claude setup-token` printed");
    }
    claude_token::save(path, &token)?;
    let kept = claude_token::load(path)?.context("the token wasn't kept")?;
    if json {
        return style::print_json(&report(path, Some(&kept), None));
    }
    let replaced = before.is_some_and(|b| b.token != kept.token);
    println!(
        "{} {name} keeps your {}Claude token ({}) in {}, which only you can read.",
        style::accent("✓"),
        if replaced { "new " } else { "" },
        masked(&kept.token),
        path.display()
    );
    let (installed, running) = service::status();
    if !HostConfig::path().exists() {
        println!(
            "  {}",
            style::dim(
                "This machine isn't a runner yet: `fctl host install` makes it one, which then keeps the token for your sessions."
            )
        );
    } else if installed && !running {
        println!(
            "  {}",
            style::amber("The host service isn't running: start it with `fctl host restart`.")
        );
    } else {
        println!(
            "  {}",
            style::dim(&format!(
                "{} tells fctl within 15 seconds that it keeps one, never what it is.",
                cfg.name
            ))
        );
    }
    if replaced {
        println!(
            "  {}",
            style::dim("Sessions running here keep the token they started with until they stop.")
        );
    }
    println!(
        "  {}",
        style::dim(
            "Next, if you haven't: choose Keep the token on your runners in Settings → Claude, or run `fctl claude connect --on-runners`."
        )
    );
    Ok(())
}

async fn status(path: &Path, name: &str, json: bool) -> Result<()> {
    let kept = claude_token::load(path);
    // What the control plane knows, as this machine's runner.
    let runner = match (HostConfig::load(), HostConfig::load_token()) {
        (Ok(cfg), Ok(token)) => Some(
            fctl_client::Api::new(&cfg.server, Some(token))
                .this_runner()
                .await
                .map_err(anyhow::Error::from),
        ),
        _ => None,
    };
    if json {
        let kept = kept.as_ref().ok().and_then(Option::as_ref);
        let known = runner.as_ref().and_then(|r| r.as_ref().ok());
        return style::print_json(&report(path, kept, known.map(|d| d.runner.claude_token)));
    }
    let kept = match kept {
        Ok(kept) => kept,
        Err(e) => {
            println!(
                "{} {}",
                style::red("✗"),
                style::terminal_safe(&format!("{e:#}"))
            );
            println!(
                "  {}",
                style::dim("`fctl host claude set-token` keeps a token there in its place.")
            );
            return Ok(());
        }
    };
    match &kept {
        Some(k) => println!(
            "{} {name} keeps your Claude token ({}), saved {}",
            style::accent("●"),
            masked(&k.token),
            style::ago(k.saved_at)
        ),
        None => println!(
            "{} {name} keeps no Claude token. `fctl host claude set-token` keeps one.",
            style::dim("○")
        ),
    }
    println!("  file:  {}", path.display());
    if kept.is_some() && !claude_token::private(path) {
        println!(
            "  {}",
            style::amber(&format!(
                "Others on this machine can read it: chmod 600 {}",
                path.display()
            ))
        );
    }
    let Some(runner) = runner else {
        println!(
            "  fctl:  {}",
            style::dim("this machine isn't a runner yet (`fctl host install`)")
        );
        return Ok(());
    };
    let line = match runner {
        Ok(d) => known_line(&d.runner.name, d.runner.claude_token, kept.as_ref()),
        Err(e) => style::dim(&format!("could not ask: {e:#}")),
    };
    println!("  fctl:  {line}");
    Ok(())
}

/// What the control plane knows of the token, against what is kept here.
fn known_line(runner: &str, known: Option<RunnerClaudeToken>, kept: Option<&Kept>) -> String {
    let soon = "the host tells it within 15 seconds while it runs";
    match (known.map(|k| k.saved_at), kept) {
        (None, _) => style::amber(&format!(
            "doesn't know whether {runner} keeps one: its host hasn't connected since it could keep one. Restart it with `fctl host restart`."
        )),
        (Some(Some(at)), Some(k)) if at == k.saved_at => {
            format!("knows {runner} keeps one, and never what it is")
        }
        (Some(Some(_)), Some(_)) => {
            style::dim(&format!("knows {runner} keeps an earlier one; {soon}"))
        }
        (Some(Some(_)), None) => style::dim(&format!("still thinks {runner} keeps one; {soon}")),
        (Some(None), Some(_)) => {
            style::dim(&format!("doesn't know {runner} keeps one yet; {soon}"))
        }
        (Some(None), None) => format!("knows {runner} keeps none"),
    }
}

/// `--json`: where the token is kept, whether it is, and what the control
/// plane knows (`fctl`, absent when it couldn't be asked). Never the token.
fn report(
    path: &Path,
    kept: Option<&Kept>,
    known: Option<Option<RunnerClaudeToken>>,
) -> serde_json::Value {
    serde_json::json!({
        "file": path.display().to_string(),
        "kept": kept.is_some(),
        "saved_at": kept.map(|k| k.saved_at),
        "masked": kept.map(|k| masked(&k.token)),
        "fctl": known.map(|k| serde_json::json!({ "claude_token": k })),
    })
}

fn remove_token(path: &Path, name: &str, json: bool) -> Result<()> {
    let removed = claude_token::remove(path)?;
    if json {
        return style::print_json(&serde_json::json!({ "removed": removed }));
    }
    if !removed {
        println!("{name} keeps no Claude token.");
        return Ok(());
    }
    println!(
        "{} Removed your Claude token from {name}.",
        style::accent("✓")
    );
    println!(
        "  {}",
        style::dim(
            "Sessions running here keep it until they stop. New runs of your sessions on the Claude connection you keep on your runners wait for one of them that keeps it."
        )
    );
    println!(
        "  {}",
        style::dim(
            "Removing it here doesn't revoke it: it works until it expires, a year after `claude setup-token` made it."
        )
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(saved_at: i64) -> Kept {
        Kept {
            token: "sk-ant-oat01-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJ-Xy3Q".into(),
            saved_at,
        }
    }

    fn known(saved_at: Option<i64>) -> Option<RunnerClaudeToken> {
        Some(RunnerClaudeToken { saved_at })
    }

    #[test]
    fn says_whether_fctl_knows_what_is_kept_here() {
        let line = |k: Option<RunnerClaudeToken>, here: Option<Kept>| {
            known_line("studio-mac", k, here.as_ref())
        };
        assert_eq!(
            line(known(Some(5)), Some(kept(5))),
            "knows studio-mac keeps one, and never what it is"
        );
        assert_eq!(line(known(None), None), "knows studio-mac keeps none");
        for (k, here, says) in [
            (known(Some(4)), Some(kept(5)), "keeps an earlier one"),
            (known(Some(4)), None, "still thinks studio-mac keeps one"),
            (
                known(None),
                Some(kept(5)),
                "doesn't know studio-mac keeps one yet",
            ),
            (None, Some(kept(5)), "`fctl host restart`"),
        ] {
            let l = line(k, here);
            assert!(l.contains(says), "{l}");
        }
    }

    #[test]
    fn reports_never_hold_the_token() -> Result<()> {
        let k = kept(5);
        let json = report(
            Path::new("/tmp/claude-token"),
            Some(&k),
            Some(known(Some(5))),
        );
        let text = serde_json::to_string(&json)?;
        assert!(!text.contains(&k.token));
        assert_eq!(json["masked"], "sk-ant-oat01-…Xy3Q");
        assert_eq!(json["fctl"]["claude_token"]["saved_at"], 5);
        assert_eq!(report(Path::new("/x"), None, None)["kept"], false);
        Ok(())
    }
}
