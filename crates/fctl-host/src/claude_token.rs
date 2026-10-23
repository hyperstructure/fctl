//! The Claude token a host keeps for its runner's owner (`fctl host claude
//! set-token`): a token from `claude setup-token` in `claude-token`, next
//! to `host.toml` (or where `claude_token_file` there says), that only its
//! owner may read (mode 0600).
//!
//! The control plane never gets it. Hellos and heartbeats say only whether
//! the host keeps one and since when ([`announce`]); the control plane
//! sends the sessions of a Claude connection whose token its owner keeps on
//! their runners only to those of their runners that keep one, with an
//! empty token that the host fills in ([`fill`]). What agents send to the
//! control plane goes through [`redact`] first, so that a token an agent
//! prints (`printenv`, say) doesn't reach the transcript.

use anyhow::{Context, Result, bail};
use fctl_proto::Millis;
use fctl_proto::runner::{ClaudeCredential, RunnerClaudeToken};
use std::borrow::Cow;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

/// What a redacted token reads as.
pub const REDACTED: &str = "[Claude token, redacted by fctl]";

/// Shorter secrets aren't redacted: they could be words. Claude's tokens
/// are about a hundred characters long.
const MIN_REDACTED: usize = 32;

/// A token the host keeps, and when it was saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    pub token: String,
    /// When the file was last written (ms since the epoch).
    pub saved_at: Millis,
}

/// The token kept at `path`: `None` without one. A file that holds
/// something other than a token from `claude setup-token` is an error, so
/// that the host doesn't say it keeps one.
pub fn load(path: &Path) -> Result<Option<Kept>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let token = text.trim();
    if token.is_empty() {
        return Ok(None);
    }
    if let Some(problem) = fctl_domain::claude::setup_token_problem(token) {
        bail!(
            "{} doesn't hold a token from `claude setup-token`: {problem}",
            path.display()
        );
    }
    let saved_at = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    Ok(Some(Kept {
        token: token.to_owned(),
        saved_at,
    }))
}

/// Keeps `token` at `path`, readable only by its owner, in place of the
/// one kept there before.
pub fn save(path: &Path, token: &str) -> Result<()> {
    let token = token.trim();
    if let Some(problem) = fctl_domain::claude::setup_token_problem(token) {
        bail!("{problem}");
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::config::write_private(path, &format!("{token}\n"))
        .with_context(|| format!("writing {}", path.display()))
}

/// Removes the token kept at `path`: whether there was one.
pub fn remove(path: &Path) -> Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// What hellos and heartbeats say of the token kept at `path`: when it was
/// saved, or nothing saved without one. A file that can't be read, or
/// holds no token, counts as none, and says why in the log.
pub fn announce(path: &Path) -> RunnerClaudeToken {
    match load(path) {
        Ok(kept) => RunnerClaudeToken {
            saved_at: kept.map(|k| k.saved_at),
        },
        Err(e) => {
            tracing::warn!("{e:#}; this runner says it keeps no Claude token");
            RunnerClaudeToken::default()
        }
    }
}

/// Puts the token kept at `path` into `cred` when it is one whose owner
/// keeps it on their runners (`on_runners`, which the control plane sends
/// without a token), or says why it can't: `runner` names this runner.
/// Other credentials stay as they are.
pub fn fill(path: &Path, cred: &mut ClaudeCredential, runner: &str) -> Result<(), String> {
    if !cred.on_runners {
        return Ok(());
    }
    match load(path) {
        Ok(Some(kept)) => {
            cred.access_token = kept.token;
            Ok(())
        }
        Ok(None) => Err(format!(
            "{runner} keeps no Claude token: run `fctl host claude set-token` on it"
        )),
        Err(e) => Err(format!("{e:#}")),
    }
}

/// `text` without the `secrets` in it: each becomes [`REDACTED`]. Secrets
/// shorter than [`MIN_REDACTED`] stay, since they could be words.
pub fn redact<'a>(text: &'a str, secrets: &[String]) -> Cow<'a, str> {
    let mut out = Cow::Borrowed(text);
    for secret in secrets.iter().filter(|s| s.len() >= MIN_REDACTED) {
        if out.contains(secret.as_str()) {
            out = Cow::Owned(out.replace(secret.as_str(), REDACTED));
        }
    }
    out
}

/// The Claude tokens a session's agent has, which [`redact`] keeps out of
/// what it sends to the control plane. Shared with the task that reads its
/// frames.
#[derive(Clone, Debug, Default)]
pub struct Secrets(Arc<Mutex<Vec<String>>>);

impl Secrets {
    /// Adds `secret`, if it is long enough to redact and new.
    pub fn add(&self, secret: &str) {
        if secret.len() < MIN_REDACTED {
            return;
        }
        let mut all = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !all.iter().any(|s| s == secret) {
            all.push(secret.to_owned());
        }
    }

    /// `text` without any of them.
    pub fn redact<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let all = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if all.is_empty() {
            return Cow::Borrowed(text);
        }
        match redact(text, &all) {
            Cow::Borrowed(_) => Cow::Borrowed(text),
            Cow::Owned(s) => Cow::Owned(s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::CredentialId;
    use fctl_proto::runner::ClaudeCredentialKind;

    const TOKEN: &str = "sk-ant-oat01-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJ-Xy3Q";

    fn credential(on_runners: bool) -> ClaudeCredential {
        ClaudeCredential {
            id: CredentialId("crd_1".into()),
            kind: ClaudeCredentialKind::SetupToken,
            access_token: String::new(),
            scopes: vec!["user:inference".into()],
            expires_at: None,
            subscription_type: None,
            account: None,
            on_runners,
        }
    }

    #[test]
    fn keeps_a_token_only_its_owner_reads() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("claude-token");
        assert_eq!(load(&path)?, None);
        assert!(!announce(&path).kept());
        save(&path, &format!("  {TOKEN}\n"))?;
        let kept = load(&path)?.context("a token")?;
        assert_eq!(kept.token, TOKEN);
        assert!(kept.saved_at > 0);
        assert_eq!(announce(&path).saved_at, Some(kept.saved_at));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)?.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert!(remove(&path)?);
        assert!(!remove(&path)?);
        assert_eq!(load(&path)?, None);
        Ok(())
    }

    #[test]
    fn keeps_only_tokens_from_claude_setup_token() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("claude-token");
        let key = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCDEF";
        let refused = save(&path, key).err().map(|e| e.to_string());
        assert!(refused.is_some_and(|e| e.contains("API key")));
        assert!(!path.exists());
        // Something else written there by hand isn't announced.
        std::fs::write(&path, "not a token\n")?;
        assert!(load(&path).is_err());
        assert!(!announce(&path).kept());
        std::fs::write(&path, "\n")?;
        assert_eq!(load(&path)?, None);
        Ok(())
    }

    #[test]
    fn fills_in_only_tokens_kept_on_runners() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("claude-token");
        let mut theirs = credential(true);
        assert_eq!(
            fill(&path, &mut theirs, "studio-mac"),
            Err("studio-mac keeps no Claude token: run `fctl host claude set-token` on it".into())
        );
        assert_eq!(theirs.access_token, "");
        save(&path, TOKEN)?;
        assert_eq!(fill(&path, &mut theirs, "studio-mac"), Ok(()));
        assert_eq!(theirs.access_token, TOKEN);
        // A credential the control plane sent with its token keeps it.
        let mut stored = ClaudeCredential {
            access_token: "sk-ant-oat01-from-the-control-plane".into(),
            ..credential(false)
        };
        assert_eq!(fill(&path, &mut stored, "studio-mac"), Ok(()));
        assert_eq!(stored.access_token, "sk-ant-oat01-from-the-control-plane");
        Ok(())
    }

    #[test]
    fn redacts_tokens_wherever_they_show() {
        let secrets = Secrets::default();
        let line = format!(
            r#"{{"type":"events","events":[{{"text":"CLAUDE_CODE_OAUTH_TOKEN={TOKEN}\nPATH=/usr/bin"}}]}}"#
        );
        assert_eq!(secrets.redact(&line), line);
        secrets.add(TOKEN);
        secrets.add(TOKEN);
        // Short ones could be words.
        secrets.add("a");
        let redacted = secrets.redact(&line);
        assert_eq!(
            redacted,
            format!(
                r#"{{"type":"events","events":[{{"text":"CLAUDE_CODE_OAUTH_TOKEN={REDACTED}\nPATH=/usr/bin"}}]}}"#
            )
        );
        assert!(serde_json::from_str::<serde_json::Value>(&redacted).is_ok());
        assert!(matches!(secrets.redact("nothing here"), Cow::Borrowed(_)));
        assert_eq!(
            redact(&format!("{TOKEN} and {TOKEN}"), &[TOKEN.to_owned()]),
            format!("{REDACTED} and {REDACTED}")
        );
    }
}
