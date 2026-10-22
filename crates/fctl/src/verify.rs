//! Confirming it's you before sensitive actions, where the instance asks
//! for it (docs/architecture.md#confirming-its-you). When the server
//! answers that an action needs it, fctl asks for a code from your
//! authenticator app, or sends you to the browser to use your passkey, and
//! sends the request again. The CLI then counts as confirmed for 15
//! minutes.

use crate::style;
use fctl_client::Api;
use fctl_client::api::Verifier;
use fctl_client::http::Error as ClientError;
use fctl_proto::api::{DevicePollResponse, SecondFactor, Verification};
use fctl_proto::{ApiError, ErrorCode};
use std::io::{BufRead, IsTerminal, Write};
use std::sync::Arc;
use std::time::Duration;

/// What the CLI's clients ask when an action needs the person to confirm
/// it's them: in a terminal, they do; without one, the server's answer is
/// the error, and `main` says how to confirm first.
pub fn verifier() -> Verifier {
    Arc::new(|api, error| Box::pin(async move { confirm(&api, Some(&error)).await }))
}

/// Asks the person to confirm it's them, in a terminal: a code, or their
/// passkey in the browser. Whether they did. `error` is what the server
/// answered the action that needs it.
pub async fn confirm(api: &Api, error: Option<&ApiError>) -> fctl_client::http::Result<bool> {
    if !std::io::stdin().is_terminal() {
        return Ok(false);
    }
    let status = api.verification().await?;
    let passkey = status.methods.contains(&SecondFactor::Passkey);
    let codes = status
        .methods
        .iter()
        .any(|m| matches!(m, SecondFactor::Totp | SecondFactor::RecoveryCode));
    if !passkey && !codes {
        return Ok(false);
    }
    if let Some(e) = error {
        eprintln!("{}", style::amber(&e.message));
    }
    if codes {
        let prompt = if passkey {
            "Code from your authenticator app or a recovery code, or Enter to use your passkey in the browser: "
        } else {
            "Code from your authenticator app, or a recovery code: "
        };
        let mut tries = 0;
        loop {
            let code = ask(prompt)?;
            if code.is_empty() {
                if passkey {
                    break;
                }
                return Ok(false);
            }
            match api.verify(&code).await {
                Ok(v) => {
                    confirmed(&v);
                    return Ok(true);
                }
                Err(ClientError::Api(e))
                    if e.code == ErrorCode::Validation
                        && e.field.as_deref() == Some("code")
                        && tries < 2 =>
                {
                    tries += 1;
                    eprintln!("{}", style::amber(&e.message));
                }
                Err(e) => return Err(e),
            }
        }
    }
    in_browser(api).await
}

/// Asks on stderr, so that `--json` output stays as it is.
fn ask(prompt: &str) -> fctl_client::http::Result<String> {
    let io = |e: std::io::Error| ClientError::Other(e.to_string());
    eprint!("{prompt}");
    std::io::stderr().flush().map_err(io)?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).map_err(io)?;
    Ok(line.trim().to_owned())
}

/// The passkey works in the browser: the person approves a code there,
/// which confirms this CLI, as they approved its sign-in.
async fn in_browser(api: &Api) -> fctl_client::http::Result<bool> {
    let client = format!("fctl on {}", fctl_host::config::hostname());
    let start = api.verify_in_browser(&client).await?;
    // As `fctl login`: approving it on another site would confirm it there.
    if !fctl_client::http::same_site(api.base(), &start.verification_uri_complete) {
        return Err(ClientError::Other(format!(
            "{} sent a link to confirm it's you on another site; nothing was confirmed",
            api.base()
        )));
    }
    eprintln!("Confirm it's you in your browser, with your passkey:");
    eprintln!();
    eprintln!("  Code: {}", style::accent(&start.user_code));
    eprintln!("  Link: {}", start.verification_uri_complete);
    eprintln!();
    if crate::cmd::auth::open_browser(&start.verification_uri_complete) {
        eprintln!(
            "{}",
            style::dim("Opened your browser. Approve the code there.")
        );
    }
    let mut interval = Duration::from_secs(u64::from(start.interval.max(1)));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(u64::from(start.expires_in));
    loop {
        tokio::time::sleep(interval).await;
        if tokio::time::Instant::now() > deadline {
            return Err(expired());
        }
        match api.device_poll(&start.device_code).await? {
            DevicePollResponse::Pending => {}
            DevicePollResponse::SlowDown => interval += Duration::from_secs(1),
            DevicePollResponse::Denied => {
                eprintln!(
                    "{}",
                    style::amber("You didn't approve it, so nothing changed.")
                );
                return Ok(false);
            }
            DevicePollResponse::Verified { verified_until } => {
                confirmed(&Verification {
                    required: true,
                    verified: true,
                    verified_until: Some(verified_until),
                    ..Default::default()
                });
                return Ok(true);
            }
            DevicePollResponse::Expired | DevicePollResponse::Approved { .. } => {
                return Err(expired());
            }
        }
    }
}

fn expired() -> ClientError {
    ClientError::Other("The code expired before you approved it. Run the command again.".into())
}

fn confirmed(v: &Verification) {
    eprintln!(
        "{}",
        style::accent(&format!("✓ {}", done(v, style::now_ms())))
    );
}

/// What confirming did, as of `now`.
pub fn done(v: &Verification, now: i64) -> String {
    match v.verified_until {
        Some(until) => {
            // Up: a confirmation that lasts 15 minutes says so, not 14.
            let minutes = ((until - now + 59_999) / 60_000).max(1);
            format!(
                "Confirmed: sensitive actions don't ask again on this CLI for {}.",
                if minutes == 1 {
                    "a minute".to_owned()
                } else {
                    format!("{minutes} minutes")
                }
            )
        }
        None => "Confirmed.".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_how_long_the_confirmation_lasts() {
        let at = |until| Verification {
            required: true,
            verified: true,
            verified_until: Some(until),
            ..Default::default()
        };
        assert_eq!(
            done(&at(15 * 60_000), 0),
            "Confirmed: sensitive actions don't ask again on this CLI for 15 minutes."
        );
        assert_eq!(
            done(&at(15 * 60_000 - 1500), 0),
            "Confirmed: sensitive actions don't ask again on this CLI for 15 minutes."
        );
        assert_eq!(
            done(&at(30_000), 0),
            "Confirmed: sensitive actions don't ask again on this CLI for a minute."
        );
        assert_eq!(done(&Verification::default(), 0), "Confirmed.");
    }
}
