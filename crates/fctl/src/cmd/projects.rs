//! `fctl projects …`

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::builder::BoolishValueParser;
use clap::{ArgGroup, Subcommand};
use fctl_proto::api::{
    AddProjectRepoRequest, CreateProjectRequest, CreateRepoRequest, ProjectRepoList,
    UpdateMembershipRequest, UpdateProjectRequest,
};
use fctl_proto::session::{
    FollowUps, GithubAccess, ImageBuild, Project, ProjectSettings, RepoAccess,
};
use std::io::{BufRead, IsTerminal, Write};

#[derive(Subcommand)]
pub enum ProjectsCmd {
    /// List your projects, in all your workspaces or one.
    List {
        /// Only this workspace's projects (slug or id).
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// Show one project.
    Show { project: String },
    /// Create a project in a workspace.
    Create {
        /// The project's name.
        name: String,
        /// The workspace it belongs to (default from `fctl workspaces use`,
        /// or your only one).
        #[arg(short, long)]
        workspace: Option<String>,
        /// Link an existing GitHub repository (owner/name) of an account the
        /// workspace works in.
        #[arg(long)]
        repo: Option<String>,
        /// Create a new repository with this name, in an organization the
        /// workspace works in.
        #[arg(long, conflicts_with = "repo")]
        new_repo: Option<String>,
        /// The organization to create the new repository in (default: the
        /// instance's, or the workspace's first).
        #[arg(long, requires = "new_repo")]
        owner: Option<String>,
        /// Make the new repository public.
        #[arg(long, requires = "new_repo")]
        public: bool,
        /// URL name (default: from the name).
        #[arg(long)]
        slug: Option<String>,
        /// A sentence for people (agents do not see it).
        #[arg(long)]
        description: Option<String>,
        /// Turn on claude.ai connectors in the project's sessions: agents
        /// can then use the connectors (mail, files, docs) of the person
        /// who starts a session without asking. Off by default.
        #[arg(long)]
        connectors: bool,
        /// Connectors are off already; kept so that existing scripts work.
        #[arg(long, hide = true, conflicts_with = "connectors")]
        no_connectors: bool,
    },
    /// Change a project's settings (maintainers). Sessions take a change
    /// when they next start or resume.
    #[command(
        group(ArgGroup::new("change").required(true).multiple(true).args(["credit_people", "credit_claude", "idle_timeout", "publish_screenshots", "screenshot_days", "project_memory", "retrospectives", "github", "follow_ups", "image", "build", "build_context", "setup_script", "package_caches", "env", "unset_env", "cpus", "memory"])),
        after_help = "Examples:\n  fctl projects set --credit-claude off          # no Claude trailer\n  fctl projects set -p my-app --credit-people off --credit-claude off   # no trailers at all\n  fctl projects set --idle-timeout 30            # idle sessions keep their sandbox for 30 minutes\n  fctl projects set --publish-screenshots on --screenshot-days 7\n  fctl projects set --project-memory off --retrospectives off   # agents keep neither\n  fctl projects set -p web --github comment    # its agents review and comment, and never push\n  fctl projects set --follow-ups comments,checks,conflicts   # agents come back for @fctl, failed checks and conflicts\n  fctl projects set --image ghcr.io/acme/agent:1  # sessions start from this image\n  fctl projects set --build .devcontainer/devcontainer.json  # or one built from the repository\n  fctl projects set --setup-script @setup.sh --env NODE_ENV=test\n  fctl projects set --package-caches on          # sessions on a runner share their downloads\n  fctl projects set --cpus 8 --memory 16g"
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
        /// Let agents publish screenshots for their pull requests (`fctl
        /// publish`), at links anyone who has one can open until they
        /// expire. Needs the instance's admins to allow it too. Off takes
        /// the project's published screenshots offline.
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        publish_screenshots: Option<bool>,
        /// Days a published screenshot's link works, 1 to 60 (default 30).
        #[arg(long, value_name = "DAYS", value_parser = clap::value_parser!(u32).range(1..=60))]
        screenshot_days: Option<u32>,
        /// Let agents keep the project's memory (`fctl memory`): each
        /// session's agent starts with the notes earlier agents and people
        /// left, and adds what future agents should know (on by default).
        /// Off, its notes stay for people to read.
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        project_memory: Option<bool>,
        /// Ask agents for a retrospective of each piece of work they finish
        /// (`fctl retro`): what they worked on, how, what got in the way
        /// and what a future agent should know (on by default).
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        retrospectives: Option<bool>,
        /// The most its sessions' agents may do on GitHub, and what a
        /// session gets unless it asks for less: push, comment (review,
        /// comment and resolve review threads; push nothing) or read.
        /// Running sessions get no more from their next GitHub token on.
        #[arg(long, value_enum)]
        github: Option<super::tokens::Github>,
        /// What brings agents back to the pull requests they opened, on
        /// the Claude subscription each session ran on: comments (people
        /// who can write to the repository mention @fctl or @agent, or
        /// start with "fctl:" or "agent:"), checks (a check of the newest
        /// commit fails), closed (merged or closed) and conflicts (it
        /// conflicts with its base branch). A comma-separated list of them,
        /// all, or none.
        #[arg(long, value_name = "LIST", value_parser = follow_ups_arg)]
        follow_ups: Option<FollowUps>,
        /// The image sessions start from, like ghcr.io/acme/agent:1, on
        /// runners whose owners allow it ("" for each runner's own). Only
        /// the Apple container and Docker drivers use images.
        #[arg(long, value_name = "REF", conflicts_with = "build")]
        image: Option<String>,
        /// Build the image from this Dockerfile or devcontainer.json in the
        /// project's repository, as its default branch has it ("" to stop),
        /// on runners whose owners allow builds from it.
        #[arg(long, value_name = "FILE")]
        build: Option<String>,
        /// The directory in the repository the Dockerfile builds in
        /// (default: its root; "" goes back to it).
        #[arg(long, value_name = "DIR")]
        build_context: Option<String>,
        /// The script that runs in the repository after cloning: the script
        /// itself, @FILE to read it from a file, - from stdin, or "" for none.
        #[arg(long, value_name = "SCRIPT")]
        setup_script: Option<String>,
        /// Share what package managers download (npm, pnpm, Yarn, Bun,
        /// Cargo, uv, pip, Go, Playwright) between the project's sessions on
        /// each runner, in caches the runner keeps for the project, up to
        /// the size its owner allows (max_cache_gb in host.toml).
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        package_caches: Option<bool>,
        /// Set an environment variable in the sandboxes (repeatable). Not
        /// secret: put credentials in the project's secrets.
        #[arg(long, value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// Remove an environment variable (repeatable).
        #[arg(long, value_name = "KEY")]
        unset_env: Vec<String>,
        /// CPUs per sandbox, 1 to 64 ("default" for each runner's default).
        #[arg(long, value_name = "N|default", value_parser = cpus_arg)]
        cpus: Option<OrDefault>,
        /// Memory per sandbox, like 16g ("default" for each runner's default).
        #[arg(long, value_name = "SIZE|default", value_parser = memory_arg)]
        memory: Option<OrDefault>,
    },
    /// Use this project by default for `fctl run` and `fctl sessions`.
    Use {
        /// The project (slug or id).
        project: String,
    },
    /// Put a workspace's projects in the order you want them, in lists and
    /// the app's sidebar: the ones you name first, the others after them as
    /// they were.
    #[command(
        after_help = "Examples:\n  fctl projects order web api       # these two first in their workspace\n  fctl projects order -w acme docs  # docs first in acme"
    )]
    Order {
        /// Projects of one workspace (slugs or ids), in order.
        #[arg(required = true, value_name = "PROJECT")]
        projects: Vec<String>,
        /// Their workspace (slug or id); default: the first project's.
        #[arg(short, long)]
        workspace: Option<String>,
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

/// Whether a project's sessions share their package caches, in words.
fn caches(s: &ProjectSettings) -> &'static str {
    if s.package_caches {
        "sessions on each runner share what their package managers download"
    } else {
        "each session's package managers download what it needs"
    }
}

/// Whether a project's agents publish screenshots, in words.
fn screenshots(s: &ProjectSettings) -> String {
    if s.publish_screenshots {
        let days = fctl_domain::published::days(s);
        let lasts = if days == 1 {
            "24 hours".to_owned()
        } else {
            format!("{days} days")
        };
        format!("publish screenshots, at links that work for {lasts}")
    } else {
        "don't publish screenshots".to_owned()
    }
}

/// What a project's agents keep for the people and agents after them, in
/// words.
fn learning(s: &ProjectSettings) -> &'static str {
    match (s.project_memory, s.retrospectives) {
        (true, true) => "keep the project's memory and record retrospectives",
        (true, false) => "keep the project's memory and record no retrospectives",
        (false, true) => "keep no memory and record retrospectives",
        (false, false) => "keep no memory and record no retrospectives",
    }
}

/// What a project's agents may do on GitHub, in words.
fn on_github(s: &ProjectSettings) -> &'static str {
    match s.github_access.unwrap_or_default() {
        GithubAccess::Push => "push, or less when a session asks",
        GithubAccess::Comment => "review and comment at most, and push nothing",
        GithubAccess::Read => "only read",
    }
}

/// `--follow-ups`: what brings agents back, as a list, `all` or `none`.
fn follow_ups_arg(s: &str) -> std::result::Result<FollowUps, String> {
    let mut f = FollowUps::default();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "all" => {
                f = FollowUps {
                    comments: true,
                    checks: true,
                    closed: true,
                    conflicts: true,
                }
            }
            "none" => f = FollowUps::default(),
            "comments" => f.comments = true,
            "checks" => f.checks = true,
            "closed" => f.closed = true,
            "conflicts" => f.conflicts = true,
            other => {
                return Err(format!(
                    "{other:?} isn't one: use comments, checks, closed and conflicts, all or none"
                ));
            }
        }
    }
    Ok(f)
}

/// What brings a project's agents back to their pull requests, in words.
fn follow_ups(s: &ProjectSettings) -> String {
    let f = s.follow_ups;
    let reasons: Vec<&str> = [
        (f.comments, "comments that mention them"),
        (f.checks, "failed checks"),
        (f.conflicts, "conflicts with their base branch"),
        (f.closed, "being merged or closed"),
    ]
    .into_iter()
    .filter_map(|(on, what)| on.then_some(what))
    .collect();
    match reasons.as_slice() {
        [] => "don't come back to their pull requests on their own".to_owned(),
        [one] => format!("come back to their pull requests for {one}"),
        [rest @ .., last] => format!(
            "come back to their pull requests for {} and {last}",
            rest.join(", ")
        ),
    }
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

/// A number, or `None` for `default`: each runner's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrDefault(Option<u32>);

/// `--cpus`: a number, or `default`.
fn cpus_arg(s: &str) -> std::result::Result<OrDefault, String> {
    if s.trim().eq_ignore_ascii_case("default") {
        return Ok(OrDefault(None));
    }
    match s.trim().parse::<u32>() {
        Ok(n) if (1..=64).contains(&n) => Ok(OrDefault(Some(n))),
        _ => Err("use 1 to 64 CPUs, or default".into()),
    }
}

/// `--memory`: an amount like 16g, or `default`.
fn memory_arg(s: &str) -> std::result::Result<OrDefault, String> {
    if s.trim().eq_ignore_ascii_case("default") {
        return Ok(OrDefault(None));
    }
    fctl_domain::size::parse_memory_mb(s).map(|m| OrDefault(Some(m)))
}

/// `--setup-script`: the script, `@FILE`, `-` for stdin; `""` for none.
fn setup_script(arg: &str) -> Result<Option<String>> {
    let text = if arg == "-" {
        std::io::read_to_string(std::io::stdin())?
    } else if let Some(path) = arg.strip_prefix('@') {
        std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?
    } else {
        arg.to_owned()
    };
    Ok((!text.trim().is_empty()).then_some(text))
}

/// Where a project's sessions start from, in words.
fn image(s: &ProjectSettings, repo: Option<&str>) -> String {
    match (&s.image_build, s.image.as_deref()) {
        (Some(b), _) => {
            let from = repo.unwrap_or("the project's repository");
            let context = match &b.context {
                Some(c) if !b.is_devcontainer() => format!(" (in {c}/)"),
                _ => String::new(),
            };
            format!(
                "an image built from {from}'s {}{context}, on runners that build images from {from}",
                b.file
            )
        }
        (None, Some(i)) if !fctl_domain::image::means_default(i) => {
            format!("{i}, on runners that allow it")
        }
        _ => "each runner's own image".into(),
    }
}

/// A sandbox's size as a project sets it, in words.
fn sandbox(s: &ProjectSettings) -> String {
    let cpus = s.cpus.map(fctl_domain::size::cpus);
    let memory = s
        .memory_mb
        .map(|m| format!("{} of memory", fctl_domain::size::memory(m.into())));
    match (cpus, memory) {
        (None, None) => "each runner's default size".into(),
        (Some(c), None) => format!("{c} and the runner's default memory"),
        (None, Some(m)) => format!("the runner's default CPUs and {m}"),
        (Some(c), Some(m)) => format!("{c} and {m}"),
    }
}

/// The first line of a setup script, and how many more it has.
fn script_summary(script: &str) -> String {
    let lines: Vec<&str> = script.lines().filter(|l| !l.trim().is_empty()).collect();
    match lines.as_slice() {
        [] => "none".into(),
        [one] => one.trim().to_owned(),
        [first, rest @ ..] => {
            let more = if rest.len() == 1 { "line" } else { "lines" };
            format!("{} (and {} more {more})", first.trim(), rest.len())
        }
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

/// Projects, one a line, under their workspaces when there are several, in
/// the order they come (yours, from the control plane).
fn print_projects(projects: Vec<Project>) {
    let mut workspaces: Vec<String> = Vec::new();
    for p in &projects {
        let id = p
            .workspace
            .as_ref()
            .map(|w| w.id.to_string())
            .unwrap_or_default();
        if !workspaces.contains(&id) {
            workspaces.push(id);
        }
    }
    let several = workspaces.len() > 1;
    let mut projects = projects;
    // Together under each, should a control plane send them mixed.
    projects.sort_by_key(|p| {
        let id = p
            .workspace
            .as_ref()
            .map(|w| w.id.to_string())
            .unwrap_or_default();
        workspaces.iter().position(|w| *w == id)
    });
    let mut last: Option<String> = None;
    for p in projects {
        if several {
            let w = p
                .workspace
                .as_ref()
                .map(|w| w.name.clone())
                .unwrap_or_default();
            if last.as_deref() != Some(w.as_str()) {
                if last.is_some() {
                    println!();
                }
                println!("{}", style::dim(&w));
                last = Some(w);
            }
        }
        let repo =
            fctl_domain::repos::summary(p.repo.as_ref(), p.other_repos, p.all_repos_of.as_deref())
                .unwrap_or_else(|| "no repository".into());
        let active = if p.active_sessions > 0 {
            style::accent(&format!("{} active", p.active_sessions))
        } else {
            style::dim("idle")
        };
        println!(
            "{:<24} {:<40} {}  {}",
            style::bold(&p.slug),
            style::dim(&repo),
            active,
            style::dim(&match p.total_sessions {
                1 => "1 session".to_owned(),
                n => format!("{n} sessions"),
            })
        );
    }
}

pub async fn run(server: Option<&str>, cmd: ProjectsCmd, json: bool) -> Result<()> {
    let api = config::api(server)?;
    match cmd {
        ProjectsCmd::List { workspace } => {
            let list = match &workspace {
                Some(w) => api.workspace_projects(w).await?,
                None => api.projects().await?,
            };
            if json {
                return style::print_json(&list);
            }
            if list.projects.is_empty() {
                println!(
                    "No projects yet. Create one with `fctl projects create <name> --repo owner/name`."
                );
            }
            print_projects(list.projects);
        }
        ProjectsCmd::Order {
            projects,
            workspace,
        } => {
            let workspace = match workspace {
                Some(w) => w,
                None => {
                    let first = projects.first().map(String::as_str).unwrap_or_default();
                    api.project(first)
                        .await?
                        .workspace
                        .map(|w| w.id.to_string())
                        .context("That project has no workspace: pass --workspace")?
                }
            };
            api.update_membership(
                &workspace,
                &UpdateMembershipRequest {
                    project_order: Some(projects),
                    ..Default::default()
                },
            )
            .await?;
            let list = api.workspace_projects(&workspace).await?;
            if json {
                return style::print_json(&list);
            }
            print_projects(list.projects);
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
            if let Some(w) = &p.workspace {
                println!("in:      {} ({})", w.name, w.slug);
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
            let repo = p.repo.as_ref().map(|r| r.full_name());
            println!("image:   {}", image(&p.settings, repo.as_deref()));
            println!("size:    {}", sandbox(&p.settings));
            println!(
                "setup:   {}",
                script_summary(p.settings.setup_script.as_deref().unwrap_or_default())
            );
            println!("caches:  {}", caches(&p.settings));
            if !p.settings.env.is_empty() {
                let keys: Vec<&str> = p.settings.env.keys().map(String::as_str).collect();
                println!("env:     {}", keys.join(", "));
            }
            println!("idle:    sessions {}", idle(&p.settings));
            println!("agents:  {}", learning(&p.settings));
            println!("github:  agents {}", on_github(&p.settings));
            println!("follow:  agents {}", follow_ups(&p.settings));
            println!("role:    {}", p.role.map(|r| r.as_str()).unwrap_or("—"));
            println!("{}", style::dim(&format!("{}/p/{}", api.base(), p.slug)));
        }
        ProjectsCmd::Create {
            name,
            workspace,
            repo,
            new_repo,
            owner,
            public,
            slug,
            description,
            connectors,
            no_connectors: _,
        } => {
            let req = CreateProjectRequest {
                workspace: workspace.or_else(|| config::CliConfig::load().workspace),
                name,
                slug,
                description,
                repo,
                create_repo: new_repo.map(|n| CreateRepoRequest {
                    name: n,
                    private: !public,
                    description: None,
                    owner,
                }),
                default_branch: None,
                settings: connectors.then(|| ProjectSettings {
                    claude_ai_connectors: true,
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
            publish_screenshots,
            screenshot_days,
            project_memory,
            retrospectives,
            github,
            follow_ups: new_follow_ups,
            image: new_image,
            build,
            build_context,
            setup_script: script,
            package_caches,
            env,
            unset_env,
            cpus,
            memory,
        } => {
            let project = super::sessions::project_or_default(project)?;
            let p = api.project(&project).await?;
            // An update replaces all settings: send the others as they are.
            let mut settings = p.settings.clone();
            if let Some(i) = &new_image {
                settings.image = (!i.trim().is_empty()).then(|| i.trim().to_owned());
                settings.image_build = None;
            }
            if let Some(file) = &build {
                settings.image_build = (!file.trim().is_empty()).then(|| {
                    let file = file.trim().to_owned();
                    // A devcontainer.json says where it builds.
                    let context = settings
                        .image_build
                        .as_ref()
                        .and_then(|b| b.context.clone())
                        .filter(|_| !file.to_ascii_lowercase().ends_with(".json"));
                    ImageBuild { file, context }
                });
                if settings.image_build.is_some() {
                    settings.image = None;
                }
            }
            if let Some(dir) = &build_context {
                let Some(b) = settings.image_build.as_mut() else {
                    bail!(
                        "{} builds no image: name its Dockerfile with --build",
                        p.slug
                    );
                };
                b.context = (!dir.trim().is_empty()).then(|| dir.trim().to_owned());
            }
            if let Some(arg) = &script {
                settings.setup_script = setup_script(arg)?;
            }
            if let Some(on) = package_caches {
                settings.package_caches = on;
            }
            for pair in &env {
                let Some((k, v)) = pair.split_once('=') else {
                    bail!("--env takes KEY=VALUE, not {pair}");
                };
                settings.env.insert(k.trim().to_owned(), v.to_owned());
            }
            for k in &unset_env {
                settings.env.remove(k.trim());
            }
            if let Some(OrDefault(c)) = cpus {
                settings.cpus = c;
            }
            if let Some(OrDefault(m)) = memory {
                settings.memory_mb = m;
            }
            if let Some(on) = credit_people {
                settings.credit_people = on;
            }
            if let Some(on) = credit_claude {
                settings.credit_claude = on;
            }
            if idle_timeout.is_some() {
                settings.idle_timeout_minutes = idle_timeout;
            }
            if let Some(on) = publish_screenshots {
                settings.publish_screenshots = on;
            }
            if let Some(days) = screenshot_days {
                settings.screenshot_days = Some(days);
            }
            if let Some(on) = project_memory {
                settings.project_memory = on;
            }
            if let Some(on) = retrospectives {
                settings.retrospectives = on;
            }
            if let Some(g) = github {
                settings.github_access = Some(g.into());
            }
            if let Some(f) = new_follow_ups {
                settings.follow_ups = f;
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
            if new_image.is_some() || build.is_some() || build_context.is_some() {
                let repo = p.repo.as_ref().map(|r| r.full_name());
                println!(
                    "{} Sessions in {} start from {}",
                    style::accent("✓"),
                    style::bold(&p.slug),
                    image(&p.settings, repo.as_deref())
                );
            }
            if cpus.is_some() || memory.is_some() {
                println!(
                    "{} Sandboxes in {} get {}",
                    style::accent("✓"),
                    style::bold(&p.slug),
                    sandbox(&p.settings)
                );
            }
            if script.is_some() {
                println!(
                    "{} Setup script of {}: {}",
                    style::accent("✓"),
                    style::bold(&p.slug),
                    script_summary(p.settings.setup_script.as_deref().unwrap_or_default())
                );
            }
            if package_caches.is_some() {
                println!(
                    "{} In {}, {}",
                    style::accent("✓"),
                    style::bold(&p.slug),
                    caches(&p.settings)
                );
            }
            if !env.is_empty() || !unset_env.is_empty() {
                let keys: Vec<&str> = p.settings.env.keys().map(String::as_str).collect();
                println!(
                    "{} Environment of {}: {}",
                    style::accent("✓"),
                    style::bold(&p.slug),
                    if keys.is_empty() {
                        "no variables".to_owned()
                    } else {
                        keys.join(", ")
                    }
                );
            }
            if publish_screenshots.is_some() || screenshot_days.is_some() {
                println!(
                    "{} Agents in {} {}",
                    style::accent("✓"),
                    style::bold(&p.slug),
                    screenshots(&p.settings)
                );
                if p.settings.publish_screenshots
                    && !api.instance().await?.features.publish_screenshots
                {
                    println!(
                        "{}",
                        style::amber(
                            "This instance's admins don't allow it yet: `fctl admin set --publish-screenshots on`."
                        )
                    );
                }
            }
            if project_memory.is_some() || retrospectives.is_some() {
                println!(
                    "{} Agents in {} {}",
                    style::accent("✓"),
                    style::bold(&p.slug),
                    learning(&p.settings)
                );
            }
            if github.is_some() {
                println!(
                    "{} Agents in {} {}",
                    style::accent("✓"),
                    style::bold(&p.slug),
                    on_github(&p.settings)
                );
            }
            if new_follow_ups.is_some() {
                println!(
                    "{} Agents in {} {}",
                    style::accent("✓"),
                    style::bold(&p.slug),
                    follow_ups(&p.settings)
                );
                if p.settings.follow_ups.any() {
                    println!(
                        "{}",
                        style::dim(
                            "Only for pull requests fctl's agents opened, on the Claude subscription each session ran on."
                        )
                    );
                    if !api.instance().await?.features.github_webhook {
                        println!(
                            "{}",
                            style::amber(
                                "This instance takes no GitHub webhooks, so nothing brings them back yet: its admins set GITHUB_WEBHOOK_SECRET."
                            )
                        );
                    }
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_ups_take_a_list_all_or_none() {
        let on = |comments, checks, closed| FollowUps {
            comments,
            checks,
            closed,
            conflicts: false,
        };
        assert_eq!(follow_ups_arg("comments"), Ok(on(true, false, false)));
        assert_eq!(follow_ups_arg("Checks, closed"), Ok(on(false, true, true)));
        assert_eq!(
            follow_ups_arg("all"),
            Ok(FollowUps {
                conflicts: true,
                ..on(true, true, true)
            })
        );
        assert_eq!(
            follow_ups_arg("checks,conflicts"),
            Ok(FollowUps {
                conflicts: true,
                ..on(false, true, false)
            })
        );
        assert_eq!(follow_ups_arg("none"), Ok(on(false, false, false)));
        assert_eq!(follow_ups_arg(""), Ok(on(false, false, false)));
        assert!(follow_ups_arg("reviews").is_err());

        let said = |f: FollowUps| {
            follow_ups(&ProjectSettings {
                follow_ups: f,
                ..Default::default()
            })
        };
        assert_eq!(
            said(on(false, false, false)),
            "don't come back to their pull requests on their own"
        );
        assert_eq!(
            said(on(false, true, false)),
            "come back to their pull requests for failed checks"
        );
        assert_eq!(
            said(on(true, true, true)),
            "come back to their pull requests for comments that mention them, failed checks and being merged or closed"
        );
        assert_eq!(
            said(FollowUps {
                conflicts: true,
                ..on(false, true, true)
            }),
            "come back to their pull requests for failed checks, conflicts with their base branch and being merged or closed"
        );
    }
}
