//! `fctl token` and `fctl tokens`: personal access tokens for scripts, CI
//! and bots, limited to what they are for, and where you are signed in.

use crate::config;
use crate::style;
use anyhow::{Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use fctl_proto::api::{CreateTokenRequest, TokenAccess, TokenInfo, TokenKind, TokenScope};
use fctl_proto::session::GithubAccess;

/// What the agents of a session may do on GitHub.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Github {
    /// Push branches and open pull requests.
    Push,
    /// Review and comment on pull requests and issues; push nothing.
    Comment,
    /// Read the code, pull requests and issues; write nothing.
    Read,
}

impl From<Github> for GithubAccess {
    fn from(g: Github) -> Self {
        match g {
            Github::Push => Self::Push,
            Github::Comment => Self::Comment,
            Github::Read => Self::Read,
        }
    }
}

/// What a token does in fctl.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Access {
    /// Read projects and sessions; change nothing, start nothing.
    Read,
    /// Also start sessions, message their agents and keep projects' memory.
    Run,
    /// Everything you can do in its projects.
    Full,
}

impl From<Access> for TokenAccess {
    fn from(a: Access) -> Self {
        match a {
            Access::Read => Self::Read,
            Access::Run => Self::Run,
            Access::Full => Self::Full,
        }
    }
}

#[derive(Args)]
#[command(after_help = "Examples:
  fctl token \"deploy bot\" --days 90
  fctl token \"PR reviews\" --access run -p web --github comment --days 90
  fctl token \"Dashboard\" --access read -w acme
  FCTL_TOKEN=fctl_pat_… fctl run -p web --github comment \"Review pull request #12\"

Without --access, -p, -w and --github, it does everything you can, as you do when signed in.")]
pub struct TokenArgs {
    /// What the token is for, e.g. "PR reviews".
    pub name: String,
    /// Expire after this many days (default: never).
    #[arg(long)]
    pub days: Option<u32>,
    /// What it does: read; run (also start and steer sessions); full
    /// (everything you can). A token with any limit reaches only projects
    /// and sessions, never your account or other tokens.
    #[arg(long, value_enum)]
    pub access: Option<Access>,
    /// Only this project, by slug or id (repeat for more).
    #[arg(short, long = "project", value_name = "PROJECT")]
    pub projects: Vec<String>,
    /// Only the projects of this workspace, those made later too (repeat
    /// for more).
    #[arg(short, long = "workspace", value_name = "WORKSPACE")]
    pub workspaces: Vec<String>,
    /// The most the agents of the sessions it starts may do on GitHub:
    /// push, comment (review, comment and resolve review threads; push
    /// nothing) or read.
    #[arg(long, value_enum)]
    pub github: Option<Github>,
}

#[derive(Subcommand)]
pub enum TokensCmd {
    /// Where you are signed in, and your access tokens with their limits.
    #[command(visible_alias = "ls")]
    List,
    /// Revoke a token: sign a browser or the CLI out, or end an access
    /// token, at once.
    #[command(visible_alias = "rm")]
    Revoke {
        /// The token's id (tok_…), or an access token's name.
        token: String,
    },
}

pub async fn create(server: Option<&str>, args: TokenArgs, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let req = CreateTokenRequest {
        name: args.name,
        expires_in_days: args.days,
        access: args.access.map(Into::into),
        workspaces: args.workspaces,
        projects: args.projects,
        github_access: args.github.map(Into::into),
    };
    let created = api.create_token(&req).await?;
    // A control plane from before limits makes a token without any.
    if req.limited() && created.token.scope.is_none() {
        let _ = api.delete_token(created.token.id.as_str()).await;
        bail!(
            "{} can't limit access tokens yet, so fctl revoked the one it made. Update the instance, or create a token without limits.",
            api.base()
        );
    }
    if json {
        return style::print_json(&created);
    }
    println!("{}", created.secret);
    let what = created
        .token
        .scope
        .as_ref()
        .map_or_else(|| "Does everything you can.".to_owned(), describe);
    eprintln!("{}", style::dim(&what));
    eprintln!(
        "{}",
        style::dim("Shown once. Use it as FCTL_TOKEN or an `Authorization: Bearer` header.")
    );
    Ok(())
}

pub async fn run(server: Option<&str>, cmd: TokensCmd, json: bool) -> Result<()> {
    let api = config::api(server)?;
    match cmd {
        TokensCmd::List => {
            let list = api.tokens().await?;
            if json {
                return style::print_json(&list);
            }
            let (tokens, devices): (Vec<&TokenInfo>, Vec<&TokenInfo>) =
                list.tokens.iter().partition(|t| t.kind == TokenKind::Pat);
            println!("{}", style::bold("Access tokens"));
            if tokens.is_empty() {
                println!(
                    "  {}",
                    style::dim("None yet. `fctl token NAME` makes one for a script, CI or a bot.")
                );
            }
            for t in tokens {
                print_token(t);
            }
            println!();
            println!("{}", style::bold("Signed in"));
            for t in devices {
                print_token(t);
            }
            Ok(())
        }
        TokensCmd::Revoke { token } => {
            let list = api.tokens().await?;
            let wanted = token.trim();
            let matching: Vec<&TokenInfo> = list
                .tokens
                .iter()
                .filter(|t| t.id.as_str() == wanted || t.name.as_deref() == Some(wanted))
                .collect();
            let t = match matching.as_slice() {
                [t] => *t,
                [] => bail!("You have no token called {wanted}. `fctl tokens` lists them."),
                _ => bail!(
                    "{} tokens are called {wanted}: revoke one by its id (`fctl tokens` lists them)",
                    matching.len()
                ),
            };
            api.delete_token(t.id.as_str()).await?;
            if json {
                return style::print_json(&fctl_proto::api::Ok { ok: true });
            }
            println!("{} Revoked {}", style::accent("✓"), style::bold(&label(t)));
            Ok(())
        }
    }
}

fn label(t: &TokenInfo) -> String {
    match (&t.name, t.kind) {
        (Some(name), _) => name.clone(),
        (None, TokenKind::Web) => "a browser".into(),
        (None, _) => "the CLI".into(),
    }
}

fn print_token(t: &TokenInfo) {
    let kind = match t.kind {
        TokenKind::Pat => "access token",
        TokenKind::Cli => "fctl",
        TokenKind::Web => "browser",
    };
    let mut meta = vec![
        kind.to_owned(),
        format!(
            "active {}",
            style::ago(t.last_used_at.unwrap_or(t.created_at))
        ),
    ];
    if let (Some(expires), TokenKind::Pat) = (t.expires_at, t.kind) {
        meta.push(format!("expires {}", style::until(expires)));
    }
    if t.current {
        meta.push("this one".into());
    }
    println!(
        "  {}  {}  {}",
        style::bold(&style::terminal_safe(&label(t))),
        style::dim(t.id.as_str()),
        style::dim(&meta.join(" · "))
    );
    if t.kind == TokenKind::Pat {
        let what = t
            .scope
            .as_ref()
            .map_or_else(|| "Does everything you can.".to_owned(), describe);
        println!("    {}", style::terminal_safe(&what));
    }
}

/// What a limited token does, in a sentence: "Runs sessions in web; their
/// agents review and comment on GitHub, and push nothing."
pub fn describe(scope: &TokenScope) -> String {
    let does = match scope.access {
        TokenAccess::Read => "Reads",
        TokenAccess::Run => "Runs sessions in",
        TokenAccess::Full => "Does everything you can in",
    };
    let places = match &scope.places {
        None => "all your projects".to_owned(),
        Some(p) => {
            let mut items: Vec<String> = p
                .projects
                .iter()
                .map(|p| {
                    p.slug
                        .clone()
                        .unwrap_or_else(|| "a project that's gone".into())
                })
                .collect();
            items.extend(p.workspaces.iter().map(|w| match &w.name {
                Some(name) => format!("every project of {name}"),
                None => "a workspace you left".into(),
            }));
            words(&items).unwrap_or_else(|| "no project".to_owned())
        }
    };
    // A token that reads starts no session.
    let agents = match (scope.access, scope.github_access) {
        (TokenAccess::Read, _) => "",
        (_, Some(GithubAccess::Comment)) => {
            "; their agents review and comment on GitHub, and push nothing"
        }
        (_, Some(GithubAccess::Read)) => "; their agents only read on GitHub",
        _ => "",
    };
    format!("{does} {places}{agents}.")
}

/// "a", "a and b", "a, b and c".
fn words(items: &[String]) -> Option<String> {
    match items {
        [] => None,
        [one] => Some(one.clone()),
        [rest @ .., last] => Some(format!("{} and {last}", rest.join(", "))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::api::{TokenPlace, TokenPlaces};

    fn place(slug: &str, name: &str) -> TokenPlace {
        TokenPlace {
            id: format!("id_{slug}"),
            slug: Some(slug.into()),
            name: Some(name.into()),
            workspace: None,
        }
    }

    #[test]
    fn says_what_a_token_does_in_a_sentence() {
        let mut scope = TokenScope {
            access: TokenAccess::Run,
            places: Some(TokenPlaces {
                workspaces: vec![],
                projects: vec![place("web", "Web")],
            }),
            github_access: Some(GithubAccess::Comment),
        };
        assert_eq!(
            describe(&scope),
            "Runs sessions in web; their agents review and comment on GitHub, and push nothing."
        );
        scope.places = Some(TokenPlaces {
            workspaces: vec![place("acme", "Acme")],
            projects: vec![place("web", "Web"), place("api", "API")],
        });
        scope.github_access = Some(GithubAccess::Read);
        assert_eq!(
            describe(&scope),
            "Runs sessions in web, api and every project of Acme; their agents only read on GitHub."
        );
        let read = TokenScope {
            access: TokenAccess::Read,
            places: None,
            github_access: Some(GithubAccess::Read),
        };
        assert_eq!(describe(&read), "Reads all your projects.");
        let gone = TokenScope {
            access: TokenAccess::Full,
            places: Some(TokenPlaces {
                workspaces: vec![],
                projects: vec![TokenPlace {
                    id: "prj_1".into(),
                    ..Default::default()
                }],
            }),
            github_access: None,
        };
        assert_eq!(
            describe(&gone),
            "Does everything you can in a project that's gone."
        );
    }
}
