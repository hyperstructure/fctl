//! `fctl projects …`

use crate::config;
use crate::style;
use anyhow::Result;
use clap::Subcommand;
use fctl_proto::api::{CreateProjectRequest, CreateRepoRequest};

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
    },
    /// Use this project by default for `fctl run` and `fctl sessions`.
    Use {
        /// The project (slug or id).
        project: String,
    },
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
            println!("branch:  {}", p.default_branch);
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
                settings: None,
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
        ProjectsCmd::Use { project } => {
            let p = api.project(&project).await?;
            let mut cfg = config::CliConfig::load();
            cfg.project = Some(p.slug.clone());
            cfg.save()?;
            println!("Using {} by default", style::bold(&p.slug));
        }
    }
    Ok(())
}
