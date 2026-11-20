//! Identity tokens (docs/identity-tokens.md): short-lived OpenID Connect ID
//! tokens that a session's agent mints in its sandbox (`fctl id-token`),
//! which prove to a workspace's own systems and clouds that whoever holds
//! one acts for a session of the workspace: who it runs for, in which
//! project, on which runner, with what access. The instance signs them;
//! each workspace is an issuer of its own (`{FCTL_PUBLIC_URL}/oidc/{wsp_…}`),
//! whose discovery document and keys anyone can read.
//!
//! A project's agents may also reach clouds as themselves
//! ([`CloudAccess`]): fctl-agent keeps a token for each in a file and sets
//! the variables that the cloud's CLI and SDKs read, which trade it for
//! the cloud's own short-lived credentials.

use crate::Millis;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// A part of a token's subject (`sub`), `<part>:<value>`. Tokens name the
/// project and the person unless the agent asks for others
/// (`fctl id-token --subject`): the parts it asks for, always in this
/// order, whatever order it gives them in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SubjectPart {
    /// `workspace:wsp_…`
    Workspace,
    /// `project:prj_…`
    Project,
    /// `user:usr_…`: the person the session runs for.
    User,
    /// `session:ses_…`
    Session,
    /// `runner:rnr_…`
    Runner,
    /// `github_access:push`: what the agent may do on GitHub.
    GithubAccess,
}

impl SubjectPart {
    pub const ALL: [Self; 6] = [
        Self::Workspace,
        Self::Project,
        Self::User,
        Self::Session,
        Self::Runner,
        Self::GithubAccess,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Project => "project",
            Self::User => "user",
            Self::Session => "session",
            Self::Runner => "runner",
            Self::GithubAccess => "github_access",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == s)
    }
}

/// The clouds a project's agents reach as themselves, with identity tokens
/// that each cloud trades for its own short-lived credentials. fctl-agent
/// keeps each cloud's token fresh in a file in the sandbox and sets the
/// variables its CLI and SDKs read. Each cloud trusts the project's agents
/// only once someone sets it up to (`GET /api/v1/projects/{id}/identity`
/// says how).
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(default)]
pub struct CloudAccess {
    pub aws: Option<AwsAccess>,
    pub gcp: Option<GcpAccess>,
    pub azure: Option<AzureAccess>,
}

impl CloudAccess {
    /// Whether it reaches any cloud.
    pub fn any(&self) -> bool {
        self.aws.is_some() || self.gcp.is_some() || self.azure.is_some()
    }
}

/// An AWS role the agents assume with their tokens
/// (`AssumeRoleWithWebIdentity`, audience `sts.amazonaws.com`):
/// fctl-agent sets `AWS_ROLE_ARN`, `AWS_WEB_IDENTITY_TOKEN_FILE` and
/// `AWS_ROLE_SESSION_NAME`, which the AWS CLI and SDKs read.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AwsAccess {
    /// `arn:aws:iam::123456789012:role/fctl-agents`.
    pub role_arn: String,
    /// The region the CLI and SDKs use (`AWS_REGION`). Absent: theirs.
    pub region: Option<String>,
    /// The tokens carry session tags (`https://aws.amazon.com/tags`): the
    /// workspace, project, person, session, runner and GitHub access, for
    /// conditions on `aws:RequestTag/fctl:…` in the role's trust policy and
    /// `aws:PrincipalTag/fctl:…` in its permissions. The trust policy must
    /// then allow `sts:TagSession`, or AWS refuses the role.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub session_tags: bool,
}

/// A Google Cloud workload identity provider the agents' tokens go to
/// (its full resource URL is their audience): fctl-agent writes a
/// credential configuration and sets `GOOGLE_APPLICATION_CREDENTIALS` and
/// `CLOUDSDK_AUTH_CREDENTIAL_FILE_OVERRIDE`, which the client libraries
/// and `gcloud` read.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct GcpAccess {
    /// The provider's resource name:
    /// `projects/123456789/locations/global/workloadIdentityPools/fctl/providers/acme`.
    pub provider: String,
    /// A service account the agents act as, by impersonating it. Absent:
    /// they act as the pool's principals, which are given access
    /// themselves.
    pub service_account: Option<String>,
}

/// A Microsoft Entra application (or user-assigned managed identity) whose
/// federated credential trusts the project's agents (audience
/// `api://AzureADTokenExchange`, subject `project:prj_…`): fctl-agent sets
/// `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_FEDERATED_TOKEN_FILE` and
/// `AZURE_AUTHORITY_HOST`, which the Azure SDKs read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AzureAccess {
    /// The directory (tenant) id.
    pub tenant_id: String,
    /// The application (client) id.
    pub client_id: String,
}

/// What a session's agent proves itself with ([`crate::runner::SessionSpec::id_tokens`]):
/// present where its project lets its agents mint identity tokens.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IdTokensSpec {
    /// Its workspace's issuer, the tokens' `iss`.
    pub issuer: String,
    /// The clouds its agent reaches as itself, which fctl-agent sets up.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(as = "Option<CloudAccess>", optional))]
    pub cloud_access: CloudAccess,
}

/// `GET /api/v1/projects/{id}/identity`: how the project's agents prove
/// who they are, and how each cloud it reaches is set up to trust them.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ProjectIdentity {
    /// Its agents mint identity tokens (`ProjectSettings::id_tokens`).
    pub enabled: bool,
    /// The workspace's issuer: the tokens' `iss`, and the provider's URL
    /// in a cloud.
    pub issuer: String,
    /// `{issuer}/.well-known/openid-configuration`.
    pub discovery_url: String,
    /// `{issuer}/.well-known/jwks.json`: the keys that sign them.
    pub jwks_url: String,
    /// The subject of its agents' tokens, `project:prj_…:user:usr_…`, with
    /// `*` for the person: the pattern a cloud matches.
    pub subject: String,
    /// How long a token lasts, in seconds.
    pub lifetime_secs: u32,
    pub aws: Option<AwsSetup>,
    pub gcp: Option<GcpSetup>,
    pub azure: Option<AzureSetup>,
}

/// How an AWS account trusts a project's agents with its role.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AwsSetup {
    /// The IAM OIDC identity provider for the workspace's issuer, in the
    /// role's account: `arn:aws:iam::123456789012:oidc-provider/…`.
    pub provider_arn: String,
    /// Its audience: `sts.amazonaws.com`.
    pub audience: String,
    /// The role's trust policy, a JSON document: the project's agents
    /// may assume it.
    pub trust_policy: String,
    /// The AWS CLI commands that add the provider and the trust policy.
    pub commands: String,
}

/// How a Google Cloud workload identity pool trusts a project's agents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GcpSetup {
    /// The tokens' audience: the provider's full resource URL.
    pub audience: String,
    /// The principals of the project's agents in the pool:
    /// `principalSet://…/attribute.project_id/prj_…`.
    pub principal_set: String,
    /// The `gcloud` commands that add the provider and let the project's
    /// agents act as the service account, or say whom to give access.
    pub commands: String,
}

/// How a Microsoft Entra application trusts a project's agents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AzureSetup {
    /// The federated credential's subject: `project:prj_…`.
    pub subject: String,
    /// Its audience: `api://AzureADTokenExchange`.
    pub audience: String,
    /// The federated credential, a JSON document, as `az ad app
    /// federated-credential create --parameters` takes it.
    pub credential: String,
    /// The Azure CLI command that adds it.
    pub commands: String,
}

/// An identity token, as `fctl id-token --json` prints it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IdToken {
    /// The signed JWT.
    pub token: String,
    /// When it expires: its `exp`.
    pub expires_at: Millis,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_parts_read_as_they_are_written() {
        for part in SubjectPart::ALL {
            assert_eq!(SubjectPart::parse(part.as_str()), Some(part));
            assert_eq!(
                serde_json::to_value(part).ok(),
                Some(serde_json::Value::String(part.as_str().to_owned()))
            );
        }
        assert_eq!(SubjectPart::parse("repository"), None);
    }

    #[test]
    fn cloud_access_leaves_out_what_it_lacks() {
        let none = CloudAccess::default();
        assert!(!none.any());
        assert_eq!(
            serde_json::to_value(&none).ok(),
            Some(serde_json::json!({}))
        );
        let aws: Result<CloudAccess, _> = serde_json::from_value(serde_json::json!({
            "aws": { "role_arn": "arn:aws:iam::123456789012:role/agents" }
        }));
        let Ok(aws) = aws else {
            panic!("reads AWS alone");
        };
        assert!(aws.any());
        assert_eq!(aws.aws.as_ref().map(|a| a.session_tags), Some(false));
        assert_eq!(
            serde_json::to_value(&aws).ok(),
            Some(serde_json::json!({
                "aws": { "role_arn": "arn:aws:iam::123456789012:role/agents" }
            }))
        );
    }
}
