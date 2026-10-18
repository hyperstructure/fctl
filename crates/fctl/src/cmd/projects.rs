//! `fctl projects …`

use crate::config;
use crate::style;
use anyhow::{Result, bail};
use clap::builder::BoolishValueParser;
use clap::{ArgGroup, Subcommand};
use fctl_proto::api::{
    AddProjectRepoRequest, CreateProjectRequest, CreateRepoRequest, ProjectRepoList,
    UpdateProjectRequest,
};
use fctl_proto::session::{ProjectSettings, RepoAccess};
use std::io::{BufRead, IsTerminal, Write};

#[derive(Subcommand)]
pub enum ProjectsCmd {
    /// List your projects.
    List,
    /// Show one project.
    Show { project: String },
    /// Create a project.
    Create {
        /// The project's name.
        name: String,
        /// Link an existing GitHub repository (owner/name).
        #[arg(long)]
        repo: Option<String>,
        /// Create a new repository with this name in the app's organization.
        #[arg(long, conflicts_with = "repo")]
        new_repo: Option<String>,
        /// Make the new repository public.
        #[arg(long, requires = "new_repo")]
        public: bool,
        /// URL name (default: from the name).
        #[arg(long)]
        slug: Option<String>,
        /// A sentence for people (agents do not see it).
        #[arg(long)]
        description: Option<String>,
        /// Turn off claude.ai connectors in the project's sessions. With
        /// them, agents can use the connectors (mail, files, docs) of the
        /// person who starts a session without asking.
        #[arg(long)]
        no_connectors: bool,
    },
    /// Change a project's settings (maintainers). Sessions take a change
    /// when they next start or resume.
    #[command(
        group(ArgGroup::new("change").required(true).multiple(true).args(["credit_people", "credit_claude", "idle_timeout"])),
        after_help = "Examples:\n  fctl projects set --credit-claude off          # no Claude trailer\n  fctl projects set -p my-app --credit-people off --credit-claude off   # no trailers at all\n  fctl projects set --idle-timeout 30            # idle sessions keep their sandbox for 30 minutes"
    )]
    Set {
        /// The project (slug or id); default from `fctl projects use`.
        #[arg(short, long)]
        project: Option<String>,
        /// Credit the person who started a session in its commits, with a
        /// Co-authored-by trailer for the commit email they set (`fctl
        /// account set`).
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        credit_people: Option<bool>,
        /// Credit Claude in commits, as Claude Code does by default: its
        /// Co-authored-by trailer, and in Remote Control sessions the
        /// claude.ai session link.
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        credit_claude: Option<bool>,
        /// Minutes a session that waits for a message keeps its sandbox, and
        /// its runner's slot, before it stops (default 10). A message
        /// resumes it.
        #[arg(long, value_name = "MINUTES", value_parser = clap::value_parser!(u32).range(1..=10_080))]
        idle_timeout: Option<u32>,
    },
    /// Use this project by default for `fctl run` and `fctl sessions`.
    Use {
        /// The project (slug or id).
        project: String,
    },
    /// The repositories the project's agents may use besides its primary one.
    #[command(subcommand)]
    Repos(ReposCmd),
    /// Delete a project with its sessions and their transcripts (owners only).
    /// Its repository on GitHub stays. This cannot be undone.
    Delete {
        /// The project (slug or id).
        project: String,
        /// The project's slug, to confirm without being asked (for scripts).
        #[arg(long, value_name = "SLUG")]
        confirm: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ReposCmd {
    /// List them, the primary repository first.
    List {
        /// The project (slug or id); default from `fctl projects use`.
        #[arg(short, long)]
        project: Option<String>,
    },
    /// Let agents use a repository, or change what they may do in it.
    ///
    /// Agents clone it when they need it, in sessions that start afterwards.
    Add {
        /// owner/name, or owner/* for every repository of the account the
        /// GitHub App can reach (read only). The project's repositories all
        /// belong to one account.
        repo: String,
        /// Let agents push branches and open pull requests (default: read only).
        #[arg(long)]
        write: bool,
        /// The project (slug or id); default from `fctl projects use`.
        #[arg(short, long)]
        project: Option<String>,
    },
    /// Stop agents from using a repository.
    ///
    /// Running sessions lose it when their GitHub tokens are next refreshed,
    /// within the hour.
    Remove {
        /// owner/name, or owner/*.
        repo: String,
        /// The project (slug or id); default from `fctl projects use`.
        #[arg(short, long)]
        project: Option<String>,
    },
}

fn print_repos(list: &ProjectRepoList) {
    for r in &list.repos {
        let name = match &r.name {
            Some(n) => format!("{}/{n}", r.owner),
            None => format!("{}/*", r.owner),
        };
        let access = match (r.primary, r.access, &r.name) {
            (true, ..) => "primary, read and write".to_owned(),
            (_, RepoAccess::Write, _) => "read and write".to_owned(),
            (_, RepoAccess::Read, Some(_)) => "read only".to_owned(),
            (_, RepoAccess::Read, None) => format!("read only, every repository of {}", r.owner),
        };
        println!("{:<40} {}", style::bold(&name), style::dim(&access));
    }
}

/// When a project's idle sessions stop, in words.
fn idle(s: &ProjectSettings) -> String {
    let minutes = fctl_domain::settings::idle_timeout_secs(s) / 60;
    let unit = if minutes == 1 { "minute" } else { "minutes" };
    format!("stop after waiting {minutes} {unit} for a message; a message resumes them")
}

/// What the commits in a project's sessions credit, in words.
fn credit(s: &ProjectSettings) -> &'static str {
    match (s.credit_claude, s.credit_people) {
        (true, true) => "credit Claude, and whoever started the session if they set a commit email",
        (true, false) => "credit Claude alone",
        (false, true) => {
            "credit whoever started the session if they set a commit email, not Claude"
        }
        (false, false) => "credit nobody",
    }
}

async fn repos(api: &fctl_client::Api, cmd: ReposCmd, json: bool) -> Result<()> {
    let list = match cmd {
        ReposCmd::List { project } => {
            let project = super::sessions::project_or_default(project)?;
            let list = api.project_repos(&project).await?;
            if !json && list.repos.is_empty() {
                println!(
                    "{project} has no repositories. Add one with `fctl projects repos add owner/name -p {project}`."
                );
                return Ok(());
            }
            list
        }
        ReposCmd::Add {
            repo,
            write,
            project,
        } => {
            let project = super::sessions::project_or_default(project)?;
            let req = AddProjectRepoRequest {
                repo,
                access: Some(if write {
                    RepoAccess::Write
                } else {
                    RepoAccess::Read
                }),
            };
            api.add_project_repo(&project, &req).await?
        }
        ReposCmd::Remove { repo, project } => {
            let project = super::sessions::project_or_default(project)?;
            let (owner, name) =
                fctl_domain::repos::parse(&repo).map_err(|e| anyhow::anyhow!(e.message))?;
            api.remove_project_repo(&project, &owner, name.as_deref())
                .await?
        }
    };
    if json {
        return style::print_json(&list);
    }
    print_repos(&list);
    Ok(())
}

pub async fn run(server: Option<&str>, cmd: ProjectsCmd, json: bool) -> Result<()> {
    let api = config::api(server)?;
    match cmd {
        ProjectsCmd::List => {
            let list = api.projects().await?;
            if json {
                return style::print_json(&list);
            }
            if list.projects.is_empty() {
                println!(
                    "No projects yet. Create one with `fctl projects create <name> --repo owner/name`."
                );
            }
            for p in list.projects {
                let repo = p
                    .repo
                    .as_ref()
                    .map(|r| r.full_name())
                    .unwrap_or_else(|| "no repository".into());
                let active = if p.active_sessions > 0 {
                    style::accent(&format!("{} active", p.active_sessions))
                } else {
                    style::dim("idle")
                };
                println!(
                    "{:<24} {:<36} {}  {}",
                    style::bold(&p.slug),
                    style::dim(&repo),
                    active,
                    style::dim(&format!("{} sessions", p.total_sessions))
                );
            }
        }
        ProjectsCmd::Show { project } => {
            let p = api.project(&project).await?;
            if json {
                return style::print_json(&p);
            }
            println!(
                "{} {}",
                style::bold(&p.name),
                style::dim(&format!("({})", p.slug))
            );
            if let Some(d) = &p.description {
                println!("{d}");
            }
            println!(
                "repo:    {}",
                p.repo
                    .as_ref()
                    .map(|r| r.full_name())
                    .unwrap_or_else(|| "—".into())
            );
            let others: Vec<String> = api
                .project_repos(p.id.as_str())
                .await?
                .repos
                .into_iter()
                .filter(|r| !r.primary)
                .map(|r| {
                    let access = match r.access {
                        RepoAccess::Write => "read and write",
                        RepoAccess::Read => "read only",
                    };
                    format!(
                        "{}/{} ({access})",
                        r.owner,
                        r.name.as_deref().unwrap_or("*")
                    )
                })
                .collect();
            if !others.is_empty() {
                println!("also:    {}", others.join(", "));
            }
            println!("branch:  {}", p.default_branch);
            println!("idle:    sessions {}", idle(&p.settings));
            println!("role:    {}", p.role.map(|r| r.as_str()).unwrap_or("—"));
            println!("{}", style::dim(&format!("{}/p/{}", api.base(), p.slug)));
        }
        ProjectsCmd::Create {
            name,
            repo,
            new_repo,
            public,
            slug,
            description,
            no_connectors,
        } => {
            let req = CreateProjectRequest {
                name,
                slug,
                description,
                repo,
                create_repo: new_repo.map(|n| CreateRepoRequest {
                    name: n,
                    private: !public,
                    description: None,
                }),
                default_branch: None,
                settings: no_connectors.then(|| ProjectSettings {
                    claude_ai_connectors: false,
                    ..Default::default()
                }),
            };
            let p = api.create_project(&req).await?;
            if json {
                return style::print_json(&p);
            }
            println!(
                "{} Created {} {}",
                style::accent("✓"),
                style::bold(&p.slug),
                style::dim(&format!("{}/p/{}", api.base(), p.slug))
            );
        }
        ProjectsCmd::Repos(cmd) => repos(&api, cmd, json).await?,
        ProjectsCmd::Set {
            project,
            credit_people,
            credit_claude,
            idle_timeout,
        } => {
            let project = super::sessions::project_or_default(project)?;
            let p = api.project(&project).await?;
            // An update replaces all settings: send the others as they are.
            let mut settings = p.settings.clone();
            if let Some(on) = credit_people {
                settings.credit_people = on;
            }
            if let Some(on) = credit_claude {
                settings.credit_claude = on;
            }
            if idle_timeout.is_some() {
                settings.idle_timeout_minutes = idle_timeout;
            }
            let p = api
                .update_project(
                    p.id.as_str(),
                    &UpdateProjectRequest {
                        settings: Some(settings),
                        ..Default::default()
                    },
                )
                .await?;
            if json {
                return style::print_json(&p);
            }
            if credit_people.is_some() || credit_claude.is_some() {
                println!(
                    "{} Commits in {}'s sessions {}",
                    style::accent("✓"),
                    style::bold(&p.slug),
                    credit(&p.settings)
                );
            }
            if idle_timeout.is_some() {
                println!(
                    "{} Sessions in {} {}",
                    style::accent("✓"),
                    style::bold(&p.slug),
                    idle(&p.settings)
                );
            }
            println!(
                "{}",
                style::dim("Sessions take the change when they next start or resume.")
            );
        }
        ProjectsCmd::Use { project } => {
            let p = api.project(&project).await?;
            let mut cfg = config::CliConfig::load();
            cfg.project = Some(p.slug.clone());
            cfg.save()?;
            println!("Using {} by default", style::bold(&p.slug));
        }
        ProjectsCmd::Delete { project, confirm } => {
            let p = api.project(&project).await?;
            if p.active_sessions > 0 {
                bail!(
                    "stop the project's {} running session(s) first",
                    p.active_sessions
                );
            }
            let typed = match confirm {
                Some(slug) => slug,
                None => {
                    if !std::io::stdin().is_terminal() {
                        bail!(
                            "pass --confirm {} to delete the project without a terminal",
                            p.slug
                        );
                    }
                    println!(
                        "{} deletes {} and its {} session(s) with their transcripts.",
                        style::red("This cannot be undone:"),
                        style::bold(&p.name),
                        p.total_sessions
                    );
                    if let Some(r) = &p.repo {
                        println!("The repository {} on GitHub stays.", r.full_name());
                    }
                    print!("Type the project's slug ({}) to confirm: ", p.slug);
                    std::io::stdout().flush()?;
                    let mut line = String::new();
                    std::io::stdin().lock().read_line(&mut line)?;
                    line.trim().to_owned()
                }
            };
            if typed.trim() != p.slug {
                bail!("that is not the project's slug; nothing was deleted");
            }
            let done = api.delete_project(p.id.as_str()).await?;
            if json {
                return style::print_json(&done);
            }
            println!("Deleted {}", style::bold(&p.name));
            let mut cfg = config::CliConfig::load();
            if cfg.project.as_deref() == Some(p.slug.as_str()) {
                cfg.project = None;
                cfg.save()?;
            }
        }
    }
    Ok(())
}
