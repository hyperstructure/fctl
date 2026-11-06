//! A Claude sign-in job: the official `claude auth login` runs in a fresh
//! sandbox; the person approves on claude.ai and pastes the code into fctl;
//! the resulting full-scope login (with Remote Control) goes to the control
//! plane, which encrypts and stores it. fctl never talks to Claude's OAuth
//! endpoints for this; the official CLI does.

use crate::driver::{Driver, Running, Sandbox, SandboxSpec};
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
    let name = crate::driver::login_name(job.as_str());
    let result = match start(&ctx, &name).await {
        Ok(started) => login(started, &ctx, cmds, &send)
            .await
            .map_err(|e| (e, None)),
        // Its sandbox couldn't be made, or `claude` started in it: the
        // driver may have stopped being ready since the host last probed
        // it (Docker quit, say). It probes it again, so that the next
        // heartbeat says so (the daemon sends one at once), and the control
        // plane passes the sign-in on to another runner.
        Err(e) => {
            let unready = ctx.recheck_driver().await;
            if let Some(why) = &unready {
                tracing::warn!(
                    "the {} driver isn't ready: {why}; it can't run the sign-in",
                    ctx.driver.name()
                );
            }
            Err((e, unready.map(|_| true)))
        }
    };
    let sb = ctx.driver.handle(&name);
    let _ = ctx.driver.remove(&sb).await;
    match result {
        Ok(bundle) => send(ClaudeLoginUpdate::Succeeded {
            bundle: Box::new(bundle),
        }),
        Err((e, unready)) => send(ClaudeLoginUpdate::Failed {
            error: format!("{e:#}"),
            unready,
        }),
    }
}

/// Starts `claude auth login` in a fresh sandbox named `name`.
async fn start(ctx: &HostCtx, name: &str) -> Result<(Sandbox, Running)> {
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
        .await
        .with_context(|| format!("Starting a sandbox on {}", ctx.cfg.name))?;
    let claude = if matches!(ctx.driver, Driver::Process(_)) {
        ctx.cfg
            .claude_bin
            .clone()
            .unwrap_or_else(|| "claude".into())
    } else {
        "claude".into()
    };
    let child = ctx
        .driver
        .spawn(
            &sb,
            &format!("unset BROWSER; exec {claude} auth login --claudeai"),
        )
        .await?;
    Ok((sb, child))
}

/// Takes the sign-in from the link claude.ai shows to the login it makes.
async fn login(
    (sb, mut child): (Sandbox, Running),
    ctx: &HostCtx,
    mut cmds: mpsc::UnboundedReceiver<LoginCmd>,
    send: &(dyn Fn(ClaudeLoginUpdate) + Send + Sync),
) -> Result<ClaudeOauthBundle> {
    let stdout = child.stdout.take().context("login stdout")?;
    let stderr = child.stderr.take().context("login stderr")?;
    let mut stdin = child.stdin.take().context("login stdin")?;
    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();
    for reader in [stdout, stderr] {
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
    use crate::config::HostConfig;
    use crate::driver::container::Flavor;
    use crate::driver::fake::FakeCli;
    use std::sync::PoisonError;

    /// A host named studio on the stand-in docker CLI, whose driver was
    /// ready when it last probed it.
    async fn studio(cli: &FakeCli, data: &std::path::Path) -> Arc<HostCtx> {
        let cfg = HostConfig {
            name: "studio".into(),
            driver: "docker".into(),
            data_dir: Some(data.to_owned()),
            ..HostConfig::default()
        };
        let ctx = Arc::new(HostCtx::new(
            cfg,
            cli.host_driver(Flavor::Docker),
            "fctl_rt_test".into(),
        ));
        *ctx.drivers.lock().unwrap_or_else(PoisonError::into_inner) =
            vec![ctx.driver.probe().await];
        ctx
    }

    /// Runs a sign-in to its end: what it said.
    async fn sign_in(ctx: &Arc<HostCtx>) -> Result<Vec<ClaudeLoginUpdate>> {
        let (_cmds, rx) = mpsc::unbounded_channel();
        let (out, mut said) = mpsc::unbounded_channel();
        tokio::time::timeout(
            Duration::from_secs(60),
            run(ctx.clone(), LoginJobId("lgn_01".into()), rx, out),
        )
        .await?;
        let mut updates = Vec::new();
        while let Ok(msg) = said.try_recv() {
            match msg {
                HostToFleet::ClaudeLogin { update, .. } => updates.push(update),
                other => bail!("not a sign-in's: {other:?}"),
            }
        }
        Ok(updates)
    }

    #[tokio::test]
    async fn a_sign_in_that_finds_docker_stopped_probes_it_again_and_says_so() -> Result<()> {
        let data = tempfile::tempdir()?;
        let cli = FakeCli::new(&[], &[])?;
        let ctx = studio(&cli, data.path()).await;
        assert_eq!(ctx.driver_problem(), None);
        // Docker stopped since.
        cli.refuse(
            "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?",
        )?;
        let updates = sign_in(&ctx).await?;
        let [
            ClaudeLoginUpdate::Starting,
            ClaudeLoginUpdate::Failed { error, unready },
        ] = &updates[..]
        else {
            bail!("{updates:?}");
        };
        assert!(
            error.starts_with("Starting a sandbox on studio: ")
                && error.contains("the Docker daemon isn't running"),
            "{error}"
        );
        assert_eq!(*unready, Some(true));
        // The next heartbeat says so.
        let why = ctx.driver_problem().unwrap_or_default();
        assert!(why.starts_with("the Docker daemon isn't running"), "{why}");
        Ok(())
    }

    #[tokio::test]
    async fn a_sign_in_whose_sandbox_fails_for_another_reason_fails_as_before() -> Result<()> {
        let data = tempfile::tempdir()?;
        let cli = FakeCli::new(&[], &[])?;
        let ctx = studio(&cli, data.path()).await;
        cli.fail_runs()?;
        let updates = sign_in(&ctx).await?;
        assert!(
            matches!(
                &updates[..],
                [
                    ClaudeLoginUpdate::Starting,
                    ClaudeLoginUpdate::Failed { unready: None, .. }
                ]
            ),
            "{updates:?}"
        );
        assert_eq!(ctx.driver_problem(), None);
        Ok(())
    }

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
