//! A Claude sign-in job: the official `claude auth login` runs in a fresh
//! sandbox; the person approves on claude.ai and pastes the code into fctl;
//! the resulting full-scope login (with Remote Control) goes to the control
//! plane, which encrypts and stores it. fctl never talks to Claude's OAuth
//! endpoints for this; the official CLI does.

use crate::driver::{Driver, SandboxSpec};
use crate::session::HostCtx;
use anyhow::{Context, Result, bail};
use fctl_proto::LoginJobId;
use fctl_proto::runner::{ClaudeAccount, ClaudeLoginUpdate, ClaudeOauthBundle, HostToFleet};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

pub enum LoginCmd {
    Code(String),
    Cancel,
}

pub async fn run(
    ctx: Arc<HostCtx>,
    job: LoginJobId,
    cmds: mpsc::UnboundedReceiver<LoginCmd>,
    out: mpsc::UnboundedSender<HostToFleet>,
) {
    let send = |update: ClaudeLoginUpdate| {
        let _ = out.send(HostToFleet::ClaudeLogin {
            job_id: job.clone(),
            update,
        });
    };
    send(ClaudeLoginUpdate::Starting);
    let name = format!(
        "fctl-login-{}",
        job.as_str().to_lowercase().replace('_', "-")
    );
    let result = login(&ctx, &name, cmds, &send).await;
    let sb = ctx.driver.handle(&name);
    let _ = ctx.driver.remove(&sb).await;
    match result {
        Ok(bundle) => send(ClaudeLoginUpdate::Succeeded {
            bundle: Box::new(bundle),
        }),
        Err(e) => send(ClaudeLoginUpdate::Failed {
            error: format!("{e:#}"),
        }),
    }
}

async fn login(
    ctx: &HostCtx,
    name: &str,
    mut cmds: mpsc::UnboundedReceiver<LoginCmd>,
    send: &(dyn Fn(ClaudeLoginUpdate) + Send + Sync),
) -> Result<ClaudeOauthBundle> {
    if matches!(ctx.driver, Driver::Process(_)) && cfg!(target_os = "macos") {
        bail!(
            "On macOS, Claude sign-in needs the apple, docker or lima driver (claude stores logins in the Keychain otherwise). Use a long-lived token from `claude setup-token` instead."
        );
    }
    let spec = SandboxSpec {
        name: name.to_owned(),
        image: ctx.cfg.image(),
        fallback_image: ctx.cfg.fallback_image(),
        cpus: 1,
        memory_mb: 1024,
        cache: None,
    };
    let sb = ctx
        .driver
        .ensure(&spec, &|line: String| tracing::info!("{line}"))
        .await?;
    let claude = if matches!(ctx.driver, Driver::Process(_)) {
        ctx.cfg
            .claude_bin
            .clone()
            .unwrap_or_else(|| "claude".into())
    } else {
        "claude".into()
    };
    let mut child = ctx
        .driver
        .spawn(
            &sb,
            &format!("unset BROWSER; exec {claude} auth login --claudeai"),
        )
        .await?;
    let stdout = child.stdout.take().context("login stdout")?;
    let stderr = child.stderr.take().context("login stderr")?;
    let mut stdin = child.stdin.take().context("login stdin")?;
    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();
    for reader in [
        Box::new(stdout) as Box<dyn tokio::io::AsyncRead + Unpin + Send>,
        Box::new(stderr),
    ] {
        let tx = line_tx.clone();
        tokio::spawn(async move {
            // The CLI prompts without a newline; split on prompts too.
            let mut r = BufReader::new(reader);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match r.read_until(b'\n', &mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
                    }
                }
            }
        });
    }
    drop(line_tx);
    let url_re = regex::Regex::new(r"https://\S+/oauth/authorize\?\S+").context("regex")?;
    let mut url_sent = false;
    let mut transcript = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15 * 60);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => bail!("The sign-in timed out"),
            line = line_rx.recv() => {
                let Some(line) = line else { break };
                transcript.push_str(&line);
                if !url_sent
                    && let Some(m) = url_re.find(&transcript) {
                        send(ClaudeLoginUpdate::AwaitingCode { url: m.as_str().trim_end_matches(['.', ',', ')']).to_owned() });
                        url_sent = true;
                    }
                if transcript.contains("Login successful") {
                    break;
                }
                if line.to_lowercase().contains("login failed") || line.to_lowercase().contains("invalid code") {
                    bail!("{}", line.trim());
                }
            }
            cmd = cmds.recv() => match cmd {
                Some(LoginCmd::Code(code)) => {
                    send(ClaudeLoginUpdate::Verifying);
                    stdin.write_all(format!("{}\n", code.trim()).as_bytes()).await?;
                    stdin.flush().await?;
                }
                Some(LoginCmd::Cancel) | None => bail!("The sign-in was cancelled"),
            }
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(20), child.wait()).await;
    if !transcript.contains("Login successful") {
        bail!(
            "claude auth login did not finish: {}",
            transcript
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("no output")
        );
    }
    let creds = ctx
        .driver
        .exec(&sb, "cat \"$HOME/.claude/.credentials.json\"")
        .await
        .context("reading the new login")?;
    let config = ctx
        .driver
        .exec(&sb, "cat \"$HOME/.claude.json\" 2>/dev/null || true")
        .await
        .unwrap_or_default();
    parse_login(&creds, &config)
}

/// Builds the bundle from `~/.claude/.credentials.json` and `~/.claude.json`.
pub fn parse_login(credentials: &str, config: &str) -> Result<ClaudeOauthBundle> {
    let v: Value = serde_json::from_str(credentials).context("the credentials file is not JSON")?;
    let o = v
        .get("claudeAiOauth")
        .context("no claudeAiOauth in the credentials file")?;
    let s = |k: &str| o.get(k).and_then(Value::as_str).map(str::to_owned);
    let access_token = s("accessToken").context("no access token")?;
    let refresh_token = s("refreshToken")
        .filter(|r| !r.is_empty())
        .context("no refresh token")?;
    let scopes = o
        .get("scopes")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let account = serde_json::from_str::<Value>(config)
        .ok()
        .and_then(|c| c.get("oauthAccount").cloned())
        .map(|a| ClaudeAccount {
            account_uuid: a
                .get("accountUuid")
                .and_then(Value::as_str)
                .map(str::to_owned),
            email: a
                .get("emailAddress")
                .and_then(Value::as_str)
                .map(str::to_owned),
            organization_uuid: a
                .get("organizationUuid")
                .and_then(Value::as_str)
                .map(str::to_owned),
            organization_name: a
                .get("organizationName")
                .and_then(Value::as_str)
                .map(str::to_owned),
            display_name: a
                .get("displayName")
                .and_then(Value::as_str)
                .map(str::to_owned),
            raw: Some(a),
        });
    Ok(ClaudeOauthBundle {
        access_token,
        refresh_token,
        expires_at: o.get("expiresAt").and_then(Value::as_i64),
        scopes,
        subscription_type: s("subscriptionType"),
        rate_limit_tier: s("rateLimitTier"),
        account,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_login() -> Result<()> {
        let creds = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-a","refreshToken":"sk-ant-ort01-b","expiresAt":1760000000000,"scopes":["user:inference","user:profile"],"subscriptionType":"max"}}"#;
        let cfg = r#"{"oauthAccount":{"accountUuid":"u1","emailAddress":"ada@example.com","organizationUuid":"o1"}}"#;
        let b = parse_login(creds, cfg)?;
        assert_eq!(b.refresh_token, "sk-ant-ort01-b");
        assert_eq!(b.subscription_type.as_deref(), Some("max"));
        assert_eq!(
            b.account.and_then(|a| a.email).as_deref(),
            Some("ada@example.com")
        );
        assert!(
            parse_login(
                r#"{"claudeAiOauth":{"accessToken":"x","refreshToken":""}}"#,
                ""
            )
            .is_err()
        );
        Ok(())
    }
}
