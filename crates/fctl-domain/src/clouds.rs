//! The clouds a project's agents reach as themselves
//! (docs/identity-tokens.md#clouds): what a project's settings may say of
//! AWS, Google Cloud and Azure, what fctl-agent mints for each and the
//! variables and files it sets up in the sandbox, and how each cloud is
//! set up to trust the project's agents (`GET /api/v1/projects/{id}/identity`).
//!
//! fctl never talks to a cloud itself: each cloud's CLI and SDKs trade the
//! token fctl-agent keeps in a file for the cloud's own short-lived
//! credentials, as they do on Kubernetes and in GitHub Actions.

use crate::id_tokens;
use fctl_proto::ApiError;
use fctl_proto::identity::{
    AwsAccess, AwsSetup, AzureAccess, AzureSetup, CloudAccess, GcpAccess, GcpSetup,
    ProjectIdentity, SubjectPart,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The audience AWS's identity providers expect.
pub const AWS_AUDIENCE: &str = "sts.amazonaws.com";
/// The audience Microsoft Entra's federated credentials expect.
pub const AZURE_AUDIENCE: &str = "api://AzureADTokenExchange";
/// Where Azure's SDKs sign in, in its public cloud.
pub const AZURE_AUTHORITY: &str = "https://login.microsoftonline.com/";
/// Google's token exchange.
const GCP_STS: &str = "https://sts.googleapis.com/v1/token";

/// A cloud fctl-agent sets up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cloud {
    Aws,
    Gcp,
    Azure,
}

impl Cloud {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Aws => "aws",
            Self::Gcp => "gcp",
            Self::Azure => "azure",
        }
    }

    /// What people call it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Aws => "AWS",
            Self::Gcp => "Google Cloud",
            Self::Azure => "Azure",
        }
    }

    /// The file fctl-agent keeps its token in, in its directory.
    pub fn token_file(self) -> &'static str {
        match self {
            Self::Aws => "aws-token",
            Self::Gcp => "gcp-token",
            Self::Azure => "azure-token",
        }
    }
}

/// The file Google's client libraries and `gcloud` read their credential
/// configuration from, beside the tokens.
pub const GCP_CREDENTIALS_FILE: &str = "gcp-credentials.json";

/// An AWS role, as its ARN names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AwsRole {
    /// `aws`, `aws-cn` or `aws-us-gov`.
    pub partition: String,
    pub account: String,
    /// Its name, after its path.
    pub name: String,
}

/// The role `arn` names: `arn:aws:iam::123456789012:role/name`, or with a
/// path before the name.
pub fn aws_role(arn: &str) -> Option<AwsRole> {
    let mut parts = arn.splitn(6, ':');
    let (Some("arn"), Some(partition), Some("iam"), Some(""), Some(account), Some(resource)) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return None;
    };
    let path = resource.strip_prefix("role/")?;
    let name = path.rsplit('/').next()?;
    let allowed = |c: char| c.is_ascii_alphanumeric() || "+=,.@_-".contains(c);
    let ok = matches!(partition, "aws" | "aws-cn" | "aws-us-gov")
        && account.len() == 12
        && account.bytes().all(|b| b.is_ascii_digit())
        && !name.is_empty()
        && name.len() <= 64
        && path.len() <= 512
        && path
            .split('/')
            .all(|seg| !seg.is_empty() && seg.chars().all(allowed));
    ok.then(|| AwsRole {
        partition: partition.to_owned(),
        account: account.to_owned(),
        name: name.to_owned(),
    })
}

/// An AWS region's name: `eu-north-1`, `us-gov-west-1`.
fn aws_region(region: &str) -> bool {
    let parts: Vec<&str> = region.split('-').collect();
    region.len() <= 32
        && parts.len() >= 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
        && parts
            .last()
            .is_some_and(|p| p.bytes().all(|b| b.is_ascii_digit()))
        && parts
            .first()
            .is_some_and(|p| p.bytes().all(|b| b.is_ascii_lowercase()))
}

/// A Google Cloud workload identity provider, as its resource name has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GcpProvider {
    pub project_number: String,
    pub pool: String,
    pub provider: String,
}

impl GcpProvider {
    /// `projects/123/locations/global/workloadIdentityPools/pool/providers/provider`.
    pub fn resource(&self) -> String {
        format!(
            "projects/{}/locations/global/workloadIdentityPools/{}/providers/{}",
            self.project_number, self.pool, self.provider
        )
    }

    /// The pool's resource name.
    pub fn pool_resource(&self) -> String {
        format!(
            "projects/{}/locations/global/workloadIdentityPools/{}",
            self.project_number, self.pool
        )
    }

    /// What tokens for it say they are for: its full resource URL, which
    /// providers expect unless they choose other audiences.
    pub fn audience(&self) -> String {
        format!("https://iam.googleapis.com/{}", self.resource())
    }
}

/// The provider a resource name names, with or without
/// `//iam.googleapis.com/` or `https://iam.googleapis.com/` before it, as
/// the console and `gcloud` show it.
pub fn gcp_provider(name: &str) -> Option<GcpProvider> {
    let name = name
        .strip_prefix("https://iam.googleapis.com/")
        .or_else(|| name.strip_prefix("//iam.googleapis.com/"))
        .unwrap_or(name);
    let parts: Vec<&str> = name.split('/').collect();
    let [
        "projects",
        number,
        "locations",
        "global",
        "workloadIdentityPools",
        pool,
        "providers",
        provider,
    ] = parts.as_slice()
    else {
        return None;
    };
    // Pools' and providers' ids: 4 to 32 lowercase letters, digits and
    // dashes, starting with a letter or digit.
    let id = |s: &str| {
        (4..=32).contains(&s.len())
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !s.starts_with('-')
    };
    let ok = !number.is_empty()
        && number.len() <= 20
        && number.bytes().all(|b| b.is_ascii_digit())
        && id(pool)
        && id(provider);
    ok.then(|| GcpProvider {
        project_number: (*number).to_owned(),
        pool: (*pool).to_owned(),
        provider: (*provider).to_owned(),
    })
}

/// A Google Cloud service account's address: `agents@acme.iam.gserviceaccount.com`.
fn gcp_service_account(email: &str) -> bool {
    let Some((name, domain)) = email.split_once('@') else {
        return false;
    };
    email.len() <= 254
        && !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && domain.ends_with(".gserviceaccount.com")
        && domain
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

/// A GUID, as Microsoft Entra's ids are.
fn guid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.iter().map(|g| g.len()).eq([8, 4, 4, 4, 12])
        && groups
            .iter()
            .all(|g| g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Checks a project's cloud access as an update sends it, and tidies it:
/// what is blank goes, ids are lowercased where clouds take them so.
pub fn check(access: &CloudAccess) -> Result<CloudAccess, ApiError> {
    let aws = match &access.aws {
        Some(a) if !a.role_arn.trim().is_empty() => {
            let role_arn = a.role_arn.trim().to_owned();
            if aws_role(&role_arn).is_none() {
                return Err(ApiError::validation(
                    "settings.cloud_access.aws.role_arn",
                    "That isn't a role's ARN: arn:aws:iam::123456789012:role/fctl-agents",
                ));
            }
            let region = a
                .region
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .map(str::to_ascii_lowercase);
            if region.as_deref().is_some_and(|r| !aws_region(r)) {
                return Err(ApiError::validation(
                    "settings.cloud_access.aws.region",
                    "That isn't an AWS region: eu-north-1, us-east-1",
                ));
            }
            Some(AwsAccess {
                role_arn,
                region,
                session_tags: a.session_tags,
            })
        }
        _ => None,
    };
    let gcp = match &access.gcp {
        Some(g) if !g.provider.trim().is_empty() => {
            let Some(provider) = gcp_provider(g.provider.trim()) else {
                return Err(ApiError::validation(
                    "settings.cloud_access.gcp.provider",
                    "That isn't a workload identity provider: projects/123456789/locations/global/workloadIdentityPools/fctl/providers/acme",
                ));
            };
            let service_account = g
                .service_account
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_ascii_lowercase);
            if service_account
                .as_deref()
                .is_some_and(|s| !gcp_service_account(s))
            {
                return Err(ApiError::validation(
                    "settings.cloud_access.gcp.service_account",
                    "That isn't a service account's address: agents@my-project.iam.gserviceaccount.com",
                ));
            }
            Some(GcpAccess {
                provider: provider.resource(),
                service_account,
            })
        }
        _ => None,
    };
    let azure = match &access.azure {
        Some(a) if !(a.tenant_id.trim().is_empty() && a.client_id.trim().is_empty()) => {
            let tenant_id = a.tenant_id.trim().to_ascii_lowercase();
            let client_id = a.client_id.trim().to_ascii_lowercase();
            if !guid(&tenant_id) {
                return Err(ApiError::validation(
                    "settings.cloud_access.azure.tenant_id",
                    "That isn't a tenant id: a GUID, as the app's Overview page shows it",
                ));
            }
            if !guid(&client_id) {
                return Err(ApiError::validation(
                    "settings.cloud_access.azure.client_id",
                    "That isn't a client id: a GUID, as the app's Overview page shows it",
                ));
            }
            Some(AzureAccess {
                tenant_id,
                client_id,
            })
        }
        _ => None,
    };
    Ok(CloudAccess { aws, gcp, azure })
}

/// A token fctl-agent keeps for a cloud: what it asks for, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudToken {
    pub cloud: Cloud,
    pub audience: String,
    pub subject: Vec<SubjectPart>,
    pub aws_session_tags: bool,
}

/// The tokens fctl-agent keeps for a project's clouds. Azure's federated
/// credentials match a subject exactly, so its tokens name the project
/// alone; the others name the project and the person, which AWS's
/// policies and Google's conditions match as they choose.
pub fn tokens(access: &CloudAccess) -> Vec<CloudToken> {
    let mut out = Vec::new();
    if let Some(aws) = &access.aws {
        out.push(CloudToken {
            cloud: Cloud::Aws,
            audience: AWS_AUDIENCE.to_owned(),
            subject: Vec::new(),
            aws_session_tags: aws.session_tags,
        });
    }
    if let Some(provider) = access.gcp.as_ref().and_then(|g| gcp_provider(&g.provider)) {
        out.push(CloudToken {
            cloud: Cloud::Gcp,
            audience: provider.audience(),
            subject: Vec::new(),
            aws_session_tags: false,
        });
    }
    if access.azure.is_some() {
        out.push(CloudToken {
            cloud: Cloud::Azure,
            audience: AZURE_AUDIENCE.to_owned(),
            subject: vec![SubjectPart::Project],
            aws_session_tags: false,
        });
    }
    out
}

/// What AWS names a session's role session: `fctl-<project>-<number>`,
/// which its CloudTrail logs show.
pub fn aws_session_name(project_slug: &str, number: u32) -> String {
    let number = number.to_string();
    // At most 64 characters, of letters, digits and `+=,.@_-`.
    let room = 64 - "fctl-".len() - 1 - number.len();
    let slug: String = project_slug
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(room)
        .collect();
    format!("fctl-{slug}-{number}")
}

/// The variables a session's Claude gets for its project's clouds, with
/// the token files in `dir`.
pub fn env(
    access: &CloudAccess,
    dir: &str,
    project_slug: &str,
    number: u32,
) -> Vec<(String, String)> {
    let file = |name: &str| format!("{}/{name}", dir.trim_end_matches('/'));
    let mut env = Vec::new();
    let mut set = |k: &str, v: String| env.push((k.to_owned(), v));
    if let Some(aws) = &access.aws {
        set("AWS_ROLE_ARN", aws.role_arn.clone());
        set("AWS_WEB_IDENTITY_TOKEN_FILE", file(Cloud::Aws.token_file()));
        set(
            "AWS_ROLE_SESSION_NAME",
            aws_session_name(project_slug, number),
        );
        if let Some(region) = &aws.region {
            set("AWS_REGION", region.clone());
            set("AWS_DEFAULT_REGION", region.clone());
        }
    }
    if access.gcp.is_some() {
        set("GOOGLE_APPLICATION_CREDENTIALS", file(GCP_CREDENTIALS_FILE));
        set(
            "CLOUDSDK_AUTH_CREDENTIAL_FILE_OVERRIDE",
            file(GCP_CREDENTIALS_FILE),
        );
    }
    if let Some(azure) = &access.azure {
        set("AZURE_CLIENT_ID", azure.client_id.clone());
        set("AZURE_TENANT_ID", azure.tenant_id.clone());
        set(
            "AZURE_FEDERATED_TOKEN_FILE",
            file(Cloud::Azure.token_file()),
        );
        set("AZURE_AUTHORITY_HOST", AZURE_AUTHORITY.to_owned());
    }
    env
}

/// The credential configuration Google's client libraries and `gcloud`
/// read (an `external_account` that reads its token from `token_file`), as
/// `gcloud iam workload-identity-pools create-cred-config` writes it.
pub fn gcp_credentials(gcp: &GcpAccess, token_file: &str) -> Option<Value> {
    let provider = gcp_provider(&gcp.provider)?;
    let mut config = json!({
        "universe_domain": "googleapis.com",
        "type": "external_account",
        "audience": format!("//iam.googleapis.com/{}", provider.resource()),
        "subject_token_type": "urn:ietf:params:oauth:token-type:jwt",
        "token_url": GCP_STS,
        "credential_source": { "file": token_file, "format": { "type": "text" } },
    });
    if let (Some(sa), Some(o)) = (&gcp.service_account, config.as_object_mut()) {
        o.insert(
            "service_account_impersonation_url".to_owned(),
            json!(format!(
                "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/{sa}:generateAccessToken"
            )),
        );
    }
    Some(config)
}

/// A project's identity: its workspace's issuer, the subject of its
/// agents' tokens, and how each of its clouds is set up to trust them.
pub fn identity(
    public_url: &str,
    workspace_id: &str,
    project_id: &str,
    project_slug: &str,
    enabled: bool,
    access: &CloudAccess,
) -> ProjectIdentity {
    let issuer = id_tokens::issuer(public_url, workspace_id);
    ProjectIdentity {
        enabled,
        discovery_url: id_tokens::discovery_url(&issuer),
        jwks_url: id_tokens::jwks_url(&issuer),
        subject: id_tokens::project_subject(project_id),
        lifetime_secs: u32::try_from(id_tokens::LIFETIME_SECS).unwrap_or(600),
        aws: access
            .aws
            .as_ref()
            .and_then(|a| aws_setup(&issuer, project_id, a)),
        gcp: access
            .gcp
            .as_ref()
            .and_then(|g| gcp_setup(&issuer, workspace_id, project_id, g)),
        azure: access
            .azure
            .as_ref()
            .map(|a| azure_setup(&issuer, project_id, project_slug, a)),
        issuer,
    }
}

fn aws_setup(issuer: &str, project_id: &str, aws: &AwsAccess) -> Option<AwsSetup> {
    let role = aws_role(&aws.role_arn)?;
    let host_path = id_tokens::issuer_host_path(issuer);
    let provider_arn = format!(
        "arn:{}:iam::{}:oidc-provider/{host_path}",
        role.partition, role.account
    );
    let mut actions = vec!["sts:AssumeRoleWithWebIdentity"];
    if aws.session_tags {
        actions.push("sts:TagSession");
    }
    // In the order AWS's own policies have, which a JSON object loses.
    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct Policy<'a> {
        version: &'a str,
        statement: [Statement<'a>; 1],
    }
    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct Statement<'a> {
        effect: &'a str,
        principal: Principal<'a>,
        action: Vec<&'a str>,
        condition: Condition,
    }
    #[derive(Serialize)]
    struct Principal<'a> {
        #[serde(rename = "Federated")]
        federated: &'a str,
    }
    #[derive(Serialize)]
    struct Condition {
        #[serde(rename = "StringEquals")]
        equals: BTreeMap<String, String>,
        #[serde(rename = "StringLike")]
        like: BTreeMap<String, String>,
    }
    let policy = Policy {
        version: "2012-10-17",
        statement: [Statement {
            effect: "Allow",
            principal: Principal {
                federated: &provider_arn,
            },
            action: actions,
            condition: Condition {
                equals: BTreeMap::from([(format!("{host_path}:aud"), AWS_AUDIENCE.to_owned())]),
                like: BTreeMap::from([(
                    format!("{host_path}:sub"),
                    id_tokens::project_subject(project_id),
                )]),
            },
        }],
    };
    let trust_policy = serde_json::to_string_pretty(&policy).ok()?;
    let commands = format!(
        "# Once in the account: the workspace's identity provider.\n\
         aws iam create-open-id-connect-provider \\\n  --url {issuer} \\\n  --client-id-list {AWS_AUDIENCE}\n\n\
         # The role's trust policy (saved as trust-policy.json): the project's agents may assume it.\n\
         aws iam update-assume-role-policy \\\n  --role-name {} \\\n  --policy-document file://trust-policy.json\n",
        role.name
    );
    Some(AwsSetup {
        provider_arn,
        audience: AWS_AUDIENCE.to_owned(),
        trust_policy,
        commands,
    })
}

/// The claims a Google Cloud provider maps to attributes, for conditions
/// and principal sets.
const GCP_ATTRIBUTES: [&str; 5] = [
    "workspace_id",
    "project_id",
    "user_id",
    "github_access",
    "started_by",
];

fn gcp_setup(
    issuer: &str,
    workspace_id: &str,
    project_id: &str,
    gcp: &GcpAccess,
) -> Option<GcpSetup> {
    let provider = gcp_provider(&gcp.provider)?;
    let principal_set = format!(
        "principalSet://iam.googleapis.com/{}/attribute.project_id/{project_id}",
        provider.pool_resource()
    );
    let mapping = std::iter::once("google.subject=assertion.sub".to_owned())
        .chain(
            GCP_ATTRIBUTES
                .iter()
                .map(|a| format!("attribute.{a}=assertion.{a}")),
        )
        .collect::<Vec<_>>()
        .join(",");
    let grant = match &gcp.service_account {
        Some(sa) => format!(
            "# The project's agents act as the service account.\n\
             gcloud iam service-accounts add-iam-policy-binding {sa} \\\n  --role=roles/iam.workloadIdentityUser \\\n  --member=\"{principal_set}\"\n"
        ),
        None => format!(
            "# Give the project's agents roles where they work, as this principal set, for example:\n\
             gcloud projects add-iam-policy-binding PROJECT_ID \\\n  --role=roles/storage.objectViewer \\\n  --member=\"{principal_set}\"\n"
        ),
    };
    let commands = format!(
        "# Once: the pool, unless it exists, and a provider in it for the workspace's issuer.\n\
         gcloud iam workload-identity-pools create {} --location=global\n\
         gcloud iam workload-identity-pools providers create-oidc {} \\\n  --location=global \\\n  --issuer-uri=\"{issuer}\" \\\n  --attribute-mapping=\"{mapping}\" \\\n  --attribute-condition=\"assertion.workspace_id == '{workspace_id}'\"\n\n{grant}",
        provider.pool_resource(),
        provider.resource(),
    );
    Some(GcpSetup {
        audience: provider.audience(),
        principal_set,
        commands,
    })
}

fn azure_setup(
    issuer: &str,
    project_id: &str,
    project_slug: &str,
    azure: &AzureAccess,
) -> AzureSetup {
    let subject = format!("project:{project_id}");
    // In the order Microsoft's own documents have, which a JSON object loses.
    #[derive(Serialize)]
    struct Credential<'a> {
        name: String,
        issuer: &'a str,
        subject: &'a str,
        audiences: [&'a str; 1],
        description: String,
    }
    let credential = Credential {
        name: format!("fctl-{project_slug}"),
        issuer,
        subject: &subject,
        audiences: [AZURE_AUDIENCE],
        description: format!("fctl agents of the project {project_slug}"),
    };
    let compact = serde_json::to_string(&credential).unwrap_or_default();
    let commands = format!(
        "az ad app federated-credential create \\\n  --id {} \\\n  --parameters '{compact}'\n",
        azure.client_id
    );
    AzureSetup {
        credential: serde_json::to_string_pretty(&credential).unwrap_or_default(),
        subject,
        audience: AZURE_AUDIENCE.to_owned(),
        commands,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROLE: &str = "arn:aws:iam::123456789012:role/fctl-agents";
    const PROVIDER: &str =
        "projects/123456789/locations/global/workloadIdentityPools/fctl/providers/acme";
    const TENANT: &str = "72f988bf-86f1-41af-91ab-2d7cd011db47";
    const CLIENT: &str = "00000000-1111-2222-3333-444444444444";

    fn access() -> CloudAccess {
        CloudAccess {
            aws: Some(AwsAccess {
                role_arn: ROLE.into(),
                region: Some("eu-north-1".into()),
                session_tags: false,
            }),
            gcp: Some(GcpAccess {
                provider: PROVIDER.into(),
                service_account: Some("agents@acme-prod.iam.gserviceaccount.com".into()),
            }),
            azure: Some(AzureAccess {
                tenant_id: TENANT.into(),
                client_id: CLIENT.into(),
            }),
        }
    }

    #[test]
    fn aws_roles_are_read_from_their_arns() {
        assert_eq!(
            aws_role(ROLE),
            Some(AwsRole {
                partition: "aws".into(),
                account: "123456789012".into(),
                name: "fctl-agents".into(),
            })
        );
        assert_eq!(
            aws_role("arn:aws-us-gov:iam::123456789012:role/team/web/deployer").map(|r| r.name),
            Some("deployer".into())
        );
        for wrong in [
            "",
            "fctl-agents",
            "arn:aws:iam::123456789012:user/ada",
            "arn:aws:iam::12345:role/x",
            "arn:aws:s3:::bucket",
            "arn:aws:iam::123456789012:role/",
            "arn:aws:iam::123456789012:role/a b",
            "arn:aws:iam::123456789012:role//x",
            "arn:other:iam::123456789012:role/x",
        ] {
            assert_eq!(aws_role(wrong), None, "{wrong}");
        }
    }

    #[test]
    fn google_providers_are_read_however_they_are_written() {
        let p = gcp_provider(PROVIDER);
        assert_eq!(
            p,
            Some(GcpProvider {
                project_number: "123456789".into(),
                pool: "fctl".into(),
                provider: "acme".into(),
            })
        );
        for written in [
            format!("//iam.googleapis.com/{PROVIDER}"),
            format!("https://iam.googleapis.com/{PROVIDER}"),
        ] {
            assert_eq!(gcp_provider(&written), p);
        }
        assert_eq!(
            p.map(|p| p.audience()),
            Some(format!("https://iam.googleapis.com/{PROVIDER}"))
        );
        for wrong in [
            "projects/my-project/locations/global/workloadIdentityPools/fctl/providers/acme",
            "projects/1/locations/europe/workloadIdentityPools/fctl/providers/acme",
            "projects/1/locations/global/workloadIdentityPools/fc/providers/acme",
            "projects/1/locations/global/workloadIdentityPools/fctl/providers/ACME",
            "projects/1/locations/global/workloadIdentityPools/fctl",
        ] {
            assert_eq!(gcp_provider(wrong), None, "{wrong}");
        }
    }

    #[test]
    fn settings_are_checked_and_tidied() {
        let sent = CloudAccess {
            aws: Some(AwsAccess {
                role_arn: format!(" {ROLE} "),
                region: Some(" EU-north-1".into()),
                session_tags: true,
            }),
            gcp: Some(GcpAccess {
                provider: format!("//iam.googleapis.com/{PROVIDER}"),
                service_account: Some("".into()),
            }),
            azure: Some(AzureAccess {
                tenant_id: TENANT.to_uppercase(),
                client_id: CLIENT.into(),
            }),
        };
        let checked = check(&sent).unwrap_or_default();
        let aws = checked.aws.unwrap_or_default();
        assert_eq!(aws.role_arn, ROLE);
        assert_eq!(aws.region.as_deref(), Some("eu-north-1"));
        assert!(aws.session_tags);
        let gcp = checked.gcp.unwrap_or_default();
        assert_eq!(gcp.provider, PROVIDER);
        assert_eq!(gcp.service_account, None);
        assert_eq!(checked.azure.map(|a| a.tenant_id), Some(TENANT.to_owned()));
        // What is blank goes.
        let blank = CloudAccess {
            aws: Some(AwsAccess::default()),
            gcp: Some(GcpAccess::default()),
            azure: Some(AzureAccess::default()),
        };
        assert_eq!(check(&blank).ok(), Some(CloudAccess::default()));
        // What is wrong is refused, by its field.
        let field = |a: CloudAccess| check(&a).err().and_then(|e| e.field);
        let mut a = access();
        a.aws = Some(AwsAccess {
            role_arn: "arn:aws:iam::123456789012:user/ada".into(),
            ..AwsAccess::default()
        });
        assert_eq!(
            field(a).as_deref(),
            Some("settings.cloud_access.aws.role_arn")
        );
        let mut a = access();
        if let Some(aws) = a.aws.as_mut() {
            aws.region = Some("europe".into());
        }
        assert_eq!(
            field(a).as_deref(),
            Some("settings.cloud_access.aws.region")
        );
        let mut a = access();
        if let Some(g) = a.gcp.as_mut() {
            g.service_account = Some("ada@gmail.com".into());
        }
        assert_eq!(
            field(a).as_deref(),
            Some("settings.cloud_access.gcp.service_account")
        );
        let mut a = access();
        if let Some(az) = a.azure.as_mut() {
            az.client_id = "acme".into();
        }
        assert_eq!(
            field(a).as_deref(),
            Some("settings.cloud_access.azure.client_id")
        );
    }

    #[test]
    fn each_cloud_gets_a_token_for_its_audience() {
        let t = tokens(&access());
        assert_eq!(
            t.iter().map(|t| t.audience.as_str()).collect::<Vec<_>>(),
            [
                AWS_AUDIENCE,
                &format!("https://iam.googleapis.com/{PROVIDER}"),
                AZURE_AUDIENCE
            ]
        );
        // Azure matches the subject exactly: the project alone.
        assert_eq!(t[2].subject, [SubjectPart::Project]);
        assert!(t[0].subject.is_empty());
        assert!(tokens(&CloudAccess::default()).is_empty());
    }

    #[test]
    fn the_sandbox_gets_what_each_clouds_tools_read() {
        let env = env(&access(), "/home/agent/.fctl/state/id-tokens/", "web", 42);
        let get = |k: &str| {
            env.iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("AWS_ROLE_ARN"), Some(ROLE));
        assert_eq!(
            get("AWS_WEB_IDENTITY_TOKEN_FILE"),
            Some("/home/agent/.fctl/state/id-tokens/aws-token")
        );
        assert_eq!(get("AWS_ROLE_SESSION_NAME"), Some("fctl-web-42"));
        assert_eq!(get("AWS_REGION"), Some("eu-north-1"));
        assert_eq!(
            get("GOOGLE_APPLICATION_CREDENTIALS"),
            Some("/home/agent/.fctl/state/id-tokens/gcp-credentials.json")
        );
        assert_eq!(
            get("CLOUDSDK_AUTH_CREDENTIAL_FILE_OVERRIDE"),
            get("GOOGLE_APPLICATION_CREDENTIALS")
        );
        assert_eq!(get("AZURE_CLIENT_ID"), Some(CLIENT));
        assert_eq!(get("AZURE_TENANT_ID"), Some(TENANT));
        assert_eq!(
            get("AZURE_FEDERATED_TOKEN_FILE"),
            Some("/home/agent/.fctl/state/id-tokens/azure-token")
        );
        assert!(super::env(&CloudAccess::default(), "/x", "web", 1).is_empty());
    }

    #[test]
    fn role_session_names_fit_what_aws_takes() {
        assert_eq!(aws_session_name("web", 7), "fctl-web-7");
        let long = aws_session_name(&"a".repeat(48), u32::MAX);
        assert!(long.len() <= 64, "{long}");
        assert!(long.ends_with(&format!("-{}", u32::MAX)));
    }

    #[test]
    fn google_reads_its_token_from_the_file() {
        let Some(gcp) = access().gcp else {
            panic!("the access has Google Cloud");
        };
        let config = gcp_credentials(&gcp, "/t/gcp-token").unwrap_or_default();
        assert_eq!(config["type"], "external_account");
        assert_eq!(
            config["audience"],
            format!("//iam.googleapis.com/{PROVIDER}")
        );
        assert_eq!(config["credential_source"]["file"], "/t/gcp-token");
        assert_eq!(
            config["service_account_impersonation_url"],
            "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/agents@acme-prod.iam.gserviceaccount.com:generateAccessToken"
        );
        let direct = GcpAccess {
            service_account: None,
            ..gcp
        };
        let config = gcp_credentials(&direct, "/t/gcp-token").unwrap_or_default();
        assert!(config.get("service_account_impersonation_url").is_none());
    }

    #[test]
    fn the_aws_trust_policy_lets_in_the_projects_agents_alone() {
        let id = identity(
            "https://app.fctl.sh",
            "wsp_acme",
            "prj_web",
            "web",
            true,
            &access(),
        );
        assert_eq!(id.issuer, "https://app.fctl.sh/oidc/wsp_acme");
        assert_eq!(id.subject, "project:prj_web:user:*");
        let Some(aws) = id.aws else {
            panic!("the project reaches AWS");
        };
        assert_eq!(
            aws.provider_arn,
            "arn:aws:iam::123456789012:oidc-provider/app.fctl.sh/oidc/wsp_acme"
        );
        // Version first, as AWS's own policies have it.
        assert!(
            aws.trust_policy
                .starts_with("{\n  \"Version\": \"2012-10-17\",\n  \"Statement\"")
        );
        let policy: Value = serde_json::from_str(&aws.trust_policy).unwrap_or_default();
        assert_eq!(
            policy["Statement"][0],
            json!({
                "Effect": "Allow",
                "Principal": { "Federated": "arn:aws:iam::123456789012:oidc-provider/app.fctl.sh/oidc/wsp_acme" },
                "Action": ["sts:AssumeRoleWithWebIdentity"],
                "Condition": {
                    "StringEquals": { "app.fctl.sh/oidc/wsp_acme:aud": "sts.amazonaws.com" },
                    "StringLike": { "app.fctl.sh/oidc/wsp_acme:sub": "project:prj_web:user:*" },
                },
            })
        );
        assert!(
            aws.commands
                .contains("--url https://app.fctl.sh/oidc/wsp_acme")
        );
        assert!(aws.commands.contains("--role-name fctl-agents"));
        // Session tags take sts:TagSession, or AWS refuses the role.
        let mut tagged = access();
        if let Some(a) = tagged.aws.as_mut() {
            a.session_tags = true;
        }
        let id = identity(
            "https://app.fctl.sh",
            "wsp_acme",
            "prj_web",
            "web",
            true,
            &tagged,
        );
        let policy: Value = id
            .aws
            .and_then(|a| serde_json::from_str(&a.trust_policy).ok())
            .unwrap_or_default();
        assert_eq!(
            policy["Statement"][0]["Action"],
            json!(["sts:AssumeRoleWithWebIdentity", "sts:TagSession"])
        );
    }

    #[test]
    fn google_and_azure_trust_the_project_by_its_id() {
        let id = identity(
            "https://app.fctl.sh",
            "wsp_acme",
            "prj_web",
            "web",
            true,
            &access(),
        );
        let Some(gcp) = id.gcp else {
            panic!("the project reaches Google Cloud");
        };
        assert_eq!(
            gcp.principal_set,
            "principalSet://iam.googleapis.com/projects/123456789/locations/global/workloadIdentityPools/fctl/attribute.project_id/prj_web"
        );
        assert!(
            gcp.commands
                .contains("--issuer-uri=\"https://app.fctl.sh/oidc/wsp_acme\"")
        );
        assert!(
            gcp.commands
                .contains("assertion.workspace_id == 'wsp_acme'")
        );
        assert!(
            gcp.commands
                .contains("attribute.project_id=assertion.project_id")
        );
        assert!(gcp.commands.contains("roles/iam.workloadIdentityUser"));
        let Some(azure) = id.azure else {
            panic!("the project reaches Azure");
        };
        assert_eq!(azure.subject, "project:prj_web");
        assert!(azure.credential.starts_with("{\n  \"name\": \"fctl-web\""));
        assert_eq!(
            serde_json::from_str::<Value>(&azure.credential).unwrap_or_default(),
            json!({
                "name": "fctl-web",
                "issuer": "https://app.fctl.sh/oidc/wsp_acme",
                "subject": "project:prj_web",
                "audiences": ["api://AzureADTokenExchange"],
                "description": "fctl agents of the project web",
            })
        );
        assert!(azure.commands.contains(&format!("--id {CLIENT}")));
        // Without clouds, the issuer and the subject alone.
        let bare = identity(
            "https://app.fctl.sh",
            "wsp_acme",
            "prj_web",
            "web",
            false,
            &CloudAccess::default(),
        );
        assert!(!bare.enabled);
        assert!(bare.aws.is_none() && bare.gcp.is_none() && bare.azure.is_none());
    }
}
