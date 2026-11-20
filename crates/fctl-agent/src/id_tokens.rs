//! The clouds a session's project reaches as itself
//! (`fctl_domain::clouds`, docs/identity-tokens.md#clouds): fctl-agent keeps
//! a token for each in a file, minted through its host (`AgentCall::IdToken`)
//! a while before the one there expires, and writes Google's credential
//! configuration beside them. Each cloud's CLI and SDKs read the file as
//! the variables Claude gets name it, and trade the token for the cloud's
//! own credentials.

use anyhow::Result;
use fctl_domain::clouds::{self, Cloud, CloudToken};
use fctl_proto::ApiError;
use fctl_proto::api::{AgentAnswer, AgentCall};
use fctl_proto::identity::CloudAccess;
use std::path::{Path, PathBuf};

/// A token is minted again once less than this is left of the one in its
/// file: tokens last 10 minutes, so every 5, and a cloud's SDK always
/// reads one with 5 minutes to go.
pub const REFRESH_BEFORE_MS: i64 = 5 * 60 * 1000;
/// How long a mint that failed waits before the next try.
const RETRY_MS: i64 = 30 * 1000;
/// How long an answer may take before the request counts as lost.
const ASK_TIMEOUT_MS: i64 = 60 * 1000;

/// One cloud's token, as fctl-agent keeps it.
#[derive(Debug)]
struct Kept {
    token: CloudToken,
    path: PathBuf,
    /// When the token in its file expires: 0 before the first.
    expires_at: i64,
    /// The request in flight, and when it went.
    asking: Option<(String, i64)>,
    /// When to try again after a failure.
    retry_at: i64,
    /// The last failure was told in the transcript: the next is not, until
    /// one succeeds again.
    told: bool,
}

/// What became of an answer to one of [`CloudTokens`]' requests.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The new token is in its file.
    Kept,
    /// It failed: what to say in the transcript, the first time.
    Failed(Option<String>),
}

/// The tokens of a session's clouds.
#[derive(Debug)]
pub struct CloudTokens {
    dir: PathBuf,
    access: CloudAccess,
    kept: Vec<Kept>,
    next: u64,
}

impl CloudTokens {
    /// The tokens `access` needs, kept in `dir`; `None` when it reaches no
    /// cloud.
    pub fn new(dir: PathBuf, access: &CloudAccess) -> Option<Self> {
        let kept: Vec<Kept> = clouds::tokens(access)
            .into_iter()
            .map(|token| Kept {
                path: dir.join(token.cloud.token_file()),
                token,
                expires_at: 0,
                asking: None,
                retry_at: 0,
                told: false,
            })
            .collect();
        (!kept.is_empty()).then(|| Self {
            dir,
            access: access.clone(),
            kept,
            next: 0,
        })
    }

    /// Readies the directory, and Google's credential configuration, which
    /// reads its token from the file beside it.
    pub fn prepare(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        if let Some(gcp) = &self.access.gcp {
            let token = self.dir.join(Cloud::Gcp.token_file());
            if let Some(config) = clouds::gcp_credentials(gcp, &token.display().to_string()) {
                write_private(
                    &self.dir.join(clouds::GCP_CREDENTIALS_FILE),
                    &serde_json::to_string_pretty(&config)?,
                )?;
            }
        }
        Ok(())
    }

    /// The variables each cloud's tools read, for Claude and the setup
    /// script.
    pub fn env(&self, project_slug: &str, number: u32) -> Vec<(String, String)> {
        clouds::env(
            &self.access,
            &self.dir.display().to_string(),
            project_slug,
            number,
        )
    }

    /// The token files, for what waits for them.
    pub fn files(&self) -> Vec<PathBuf> {
        self.kept.iter().map(|k| k.path.clone()).collect()
    }

    /// What each cloud is, for the transcript: "AWS (the role arn:…)".
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(aws) = &self.access.aws {
            parts.push(format!("AWS, as the role {}", aws.role_arn));
        }
        if let Some(gcp) = &self.access.gcp {
            parts.push(match &gcp.service_account {
                Some(sa) => format!("Google Cloud, as {sa}"),
                None => format!("Google Cloud, through {}", gcp.provider),
            });
        }
        if let Some(azure) = &self.access.azure {
            parts.push(format!("Azure, as the application {}", azure.client_id));
        }
        parts.join("; ")
    }

    /// The requests due at `now`: a token for each cloud whose file has
    /// none, or one that expires within [`REFRESH_BEFORE_MS`], unless one
    /// is on its way or a failure waits to be tried again.
    pub fn due(&mut self, now: i64) -> Vec<(String, AgentCall)> {
        let mut calls = Vec::new();
        for k in &mut self.kept {
            if k.asking
                .as_ref()
                .is_some_and(|(_, at)| now - at < ASK_TIMEOUT_MS)
                || now < k.retry_at
                || k.expires_at - now > REFRESH_BEFORE_MS
            {
                continue;
            }
            self.next += 1;
            let id = format!("idt_{}_{}", k.token.cloud.as_str(), self.next);
            k.asking = Some((id.clone(), now));
            calls.push((
                id,
                AgentCall::IdToken {
                    audience: k.token.audience.clone(),
                    nonce: None,
                    subject: k.token.subject.clone(),
                    aws_session_tags: k.token.aws_session_tags,
                },
            ));
        }
        calls
    }

    /// The control plane's answer to `request_id`, if it is one of these
    /// requests: its token goes in its file.
    pub fn answer(
        &mut self,
        request_id: &str,
        answer: Option<&AgentAnswer>,
        error: Option<&ApiError>,
        now: i64,
    ) -> Option<Outcome> {
        let k = self
            .kept
            .iter_mut()
            .find(|k| k.asking.as_ref().is_some_and(|(id, _)| id == request_id))?;
        k.asking = None;
        let written = match answer {
            Some(AgentAnswer::IdToken { token, expires_at }) => {
                match write_private(&k.path, token) {
                    Ok(()) => Ok(*expires_at),
                    Err(e) => Err(format!("writing it failed: {e:#}")),
                }
            }
            _ => Err(error.map_or_else(
                || "the host answered nothing".to_owned(),
                |e| e.message.clone(),
            )),
        };
        Some(match written {
            Ok(expires_at) => {
                k.expires_at = expires_at;
                k.told = false;
                Outcome::Kept
            }
            Err(why) => {
                k.retry_at = now + RETRY_MS;
                let first = !std::mem::replace(&mut k.told, true);
                Outcome::Failed(first.then(|| {
                    format!(
                        "{}'s identity token couldn't be minted, so its CLI and SDKs can't sign in yet: {why}",
                        k.token.cloud.name()
                    )
                }))
            }
        })
    }
}

/// Writes `contents` to `path` whole, readable by its owner alone.
fn write_private(path: &Path, contents: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Waits up to `limit` for each of `files` to be there: the setup script
/// may need the clouds, whose first tokens are on their way.
pub async fn wait_for(files: &[PathBuf], limit: std::time::Duration) {
    let until = tokio::time::Instant::now() + limit;
    while files.iter().any(|f| !f.exists()) && tokio::time::Instant::now() < until {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::identity::{AwsAccess, AzureAccess, GcpAccess};

    fn access() -> CloudAccess {
        CloudAccess {
            aws: Some(AwsAccess {
                role_arn: "arn:aws:iam::123456789012:role/agents".into(),
                region: None,
                session_tags: true,
            }),
            gcp: Some(GcpAccess {
                provider: "projects/1/locations/global/workloadIdentityPools/fctl/providers/acme"
                    .into(),
                service_account: None,
            }),
            azure: Some(AzureAccess {
                tenant_id: "72f988bf-86f1-41af-91ab-2d7cd011db47".into(),
                client_id: "00000000-1111-2222-3333-444444444444".into(),
            }),
        }
    }

    fn minted(token: &str, expires_at: i64) -> AgentAnswer {
        AgentAnswer::IdToken {
            token: token.into(),
            expires_at,
        }
    }

    #[test]
    fn no_cloud_needs_nothing() {
        assert!(CloudTokens::new(PathBuf::from("/x"), &CloudAccess::default()).is_none());
    }

    #[test]
    fn each_cloud_gets_its_token_in_its_file_and_again_before_it_expires() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let Some(mut tokens) = CloudTokens::new(dir.path().join("id-tokens"), &access()) else {
            panic!("three clouds need tokens");
        };
        tokens.prepare()?;
        let config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(
            dir.path().join("id-tokens/gcp-credentials.json"),
        )?)?;
        assert_eq!(
            config["credential_source"]["file"],
            dir.path().join("id-tokens/gcp-token").display().to_string()
        );
        let now = 1_000_000;
        let calls = tokens.due(now);
        assert_eq!(calls.len(), 3);
        let AgentCall::IdToken {
            audience,
            aws_session_tags,
            ..
        } = &calls[0].1
        else {
            panic!("an identity token");
        };
        assert_eq!(audience, "sts.amazonaws.com");
        assert!(aws_session_tags);
        // Nothing more is asked while those are on their way.
        assert!(tokens.due(now + 1000).is_empty());
        for (i, (id, _)) in calls.iter().enumerate() {
            let answer = minted(&format!("token-{i}"), now + 600_000);
            assert_eq!(
                tokens.answer(id, Some(&answer), None, now),
                Some(Outcome::Kept)
            );
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("id-tokens/aws-token"))?,
            "token-0"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("id-tokens/azure-token"))?,
            "token-2"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("id-tokens/aws-token"))?
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Not again until five minutes are left; then each again.
        assert!(tokens.due(now + 290_000).is_empty());
        assert_eq!(tokens.due(now + 300_000).len(), 3);
        // Someone else's answer is none of theirs.
        assert_eq!(tokens.answer("call_1", None, None, now), None);
        Ok(())
    }

    #[test]
    fn a_failure_is_told_once_and_tried_again_soon() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let only_aws = CloudAccess {
            gcp: None,
            azure: None,
            ..access()
        };
        let Some(mut tokens) = CloudTokens::new(dir.path().to_path_buf(), &only_aws) else {
            panic!("AWS needs a token");
        };
        let now = 5_000_000;
        let refused = ApiError::forbidden("This project's agents mint no identity tokens.");
        let (id, _) = tokens.due(now).remove(0);
        let Some(Outcome::Failed(Some(told))) = tokens.answer(&id, None, Some(&refused), now)
        else {
            panic!("the first failure is told");
        };
        assert!(told.starts_with("AWS's identity token couldn't be minted"));
        assert!(told.ends_with("This project's agents mint no identity tokens."));
        assert!(tokens.due(now + 10_000).is_empty());
        let (id, _) = tokens.due(now + RETRY_MS).remove(0);
        assert_eq!(
            tokens.answer(&id, None, Some(&refused), now + RETRY_MS),
            Some(Outcome::Failed(None))
        );
        // A request the host never answers is asked again after a minute.
        let later = now + 2 * RETRY_MS;
        assert_eq!(tokens.due(later).len(), 1);
        assert!(tokens.due(later + ASK_TIMEOUT_MS - 1).is_empty());
        assert_eq!(tokens.due(later + ASK_TIMEOUT_MS).len(), 1);
        Ok(())
    }

    #[test]
    fn the_transcript_says_which_clouds() {
        let tokens = CloudTokens::new(PathBuf::from("/t"), &access());
        assert_eq!(
            tokens.map(|t| t.summary()).as_deref(),
            Some(
                "AWS, as the role arn:aws:iam::123456789012:role/agents; Google Cloud, through projects/1/locations/global/workloadIdentityPools/fctl/providers/acme; Azure, as the application 00000000-1111-2222-3333-444444444444"
            )
        );
    }
}
