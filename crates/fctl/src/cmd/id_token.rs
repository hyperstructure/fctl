//! `fctl id-token`: an agent proves to a service of its people's that a
//! request comes from its session, in its sandbox, through fctl-agent
//! (`AgentCall::IdToken`): an OpenID Connect ID token its workspace's
//! issuer signs, for one audience, which lasts 10 minutes
//! (docs/identity-tokens.md).

use crate::style;
use anyhow::{Context, Result, bail};
use clap::Args;
use fctl_proto::SessionId;
use fctl_proto::agent::{SessionReply, SessionRequest};
use fctl_proto::api::{AgentAnswer, AgentCall};
use fctl_proto::identity::SubjectPart;
use std::time::Duration;

#[derive(Args)]
pub struct IdTokenArgs {
    /// Whom the token is for, its `aud`: the address of your service, or
    /// what it asks for (sts.amazonaws.com for AWS). The service checks it.
    #[arg(short, long, value_name = "AUDIENCE")]
    pub audience: String,
    /// The parts of its subject (`sub`), in place of the project and the
    /// person: workspace, project, user, session, runner and github_access,
    /// comma-separated, in that order whatever order you give them in.
    #[arg(long, value_name = "PARTS", value_delimiter = ',', value_parser = subject_part)]
    pub subject: Vec<SubjectPart>,
    /// A value the token repeats in its `nonce`, for a service that takes
    /// each token once.
    #[arg(long)]
    pub nonce: Option<String>,
    /// Carry AWS session tags (`https://aws.amazon.com/tags`), for a role
    /// whose trust policy allows sts:TagSession.
    #[arg(long)]
    pub aws_session_tags: bool,
    /// Print what the token says, its claims, rather than the token
    /// (`--json` prints both).
    #[arg(long)]
    pub claims: bool,
}

fn subject_part(s: &str) -> std::result::Result<SubjectPart, String> {
    SubjectPart::parse(s.trim()).ok_or_else(|| {
        format!(
            "a subject's parts are {}",
            SubjectPart::ALL.map(SubjectPart::as_str).join(", ")
        )
    })
}

pub async fn run(args: IdTokenArgs, json: bool) -> Result<()> {
    let Some((socket, session)) = super::sessions::this_sandbox(None) else {
        bail!(
            "fctl id-token mints identity tokens for the session this sandbox runs, for its agent: run it in a session's sandbox. How services check them: https://fctl.sh/docs/identity-tokens"
        );
    };
    let request = SessionRequest::Call {
        session_id: Some(SessionId(session)),
        call: AgentCall::IdToken {
            audience: args.audience,
            nonce: args.nonce,
            subject: args.subject,
            aws_session_tags: args.aws_session_tags,
        },
    };
    // fctl-agent waits up to two minutes for the control plane.
    let reply = tokio::time::timeout(
        Duration::from_secs(180),
        super::sessions::ask_agent(&socket, &request),
    )
    .await
    .context("fctl-agent did not answer in time")??;
    let (token, expires_at) = match reply {
        SessionReply::Answer {
            answer: AgentAnswer::IdToken { token, expires_at },
        } => (token, expires_at),
        SessionReply::Error { error } if super::sessions::older_agent(&error.message) => {
            bail!(
                "This sandbox's fctl-agent can't mint identity tokens yet: its runner needs a newer fctl (`fctl self update` there)."
            )
        }
        SessionReply::Error { error } => bail!("{}", error.message),
        _ => bail!(
            "This sandbox's fctl-agent can't mint identity tokens yet: its runner needs a newer fctl (`fctl self update` there)."
        ),
    };
    let claims = fctl_domain::id_tokens::read_claims(&token).unwrap_or_default();
    if json {
        return style::print_json(&serde_json::json!({
            "token": token,
            "expires_at": expires_at,
            "claims": claims,
        }));
    }
    if args.claims {
        println!("{}", serde_json::to_string_pretty(&claims)?);
        return Ok(());
    }
    println!("{token}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Fctl {
        #[command(flatten)]
        id_token: IdTokenArgs,
    }

    fn parse(args: &[&str]) -> Option<IdTokenArgs> {
        Fctl::try_parse_from(["fctl"].iter().chain(args).copied())
            .ok()
            .map(|f| f.id_token)
    }

    #[test]
    fn takes_an_audience_and_the_parts_of_a_subject() {
        let Some(a) = parse(&[
            "--audience",
            "sts.amazonaws.com",
            "--subject",
            "user,project",
            "--aws-session-tags",
        ]) else {
            panic!("parses");
        };
        assert_eq!(a.audience, "sts.amazonaws.com");
        assert_eq!(a.subject, [SubjectPart::User, SubjectPart::Project]);
        assert!(a.aws_session_tags);
        assert!(!a.claims);
        // An audience is needed, and only parts there are.
        assert!(parse(&[]).is_none());
        assert!(parse(&["-a", "x", "--subject", "repository"]).is_none());
        assert!(parse(&["-a", "x", "--claims"]).is_some_and(|a| a.claims));
    }
}
