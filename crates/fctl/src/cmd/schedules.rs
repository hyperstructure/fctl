//! `fctl schedules`: a project's schedules, tasks fctl starts a session
//! for on its own, once at a set time, again and again, or each time
//! something happens on GitHub, on the Claude subscription of the person
//! each runs for. People set them up, change, pause and delete them, and
//! run one now; other tools run one with an access token (`fctl schedules
//! run`, with what they know of why).

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use fctl_client::Api;
use fctl_domain::cron::Cron;
use fctl_proto::api::{
    CreateScheduleRequest, RunScheduleRequest, Schedule, ScheduleEvent, ScheduleEventBy,
    ScheduleEventKind, ScheduleRun, ScheduleRunEvent, ScheduleRunOutcome, ScheduleState,
    ScheduleTrigger, UpdateScheduleRequest,
};
use fctl_proto::session::GithubAccess;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

#[derive(Args)]
pub struct SchedulesArgs {
    /// The project (slug or id); default from `fctl projects use`.
    #[arg(short, long, global = true)]
    pub project: Option<String>,
    #[command(subcommand)]
    pub cmd: Option<SchedulesCmd>,
}

/// When a schedule runs: a cron expression, a time to run once, an event
/// on GitHub, or only when asked.
#[derive(Args, Default)]
pub struct WhenArgs {
    /// Run again and again: a cron expression (minute, hour, day of the
    /// month, month, day of the week), like "0 9 * * 1-5", or @daily,
    /// @weekly…
    #[arg(long, conflicts_with_all = ["at", "when_asked", "on"])]
    pub cron: Option<String>,
    /// Run once: in 2h, 30m, 1d, or a time with its UTC offset, like
    /// 2026-10-30T14:00+01:00.
    #[arg(long, conflicts_with_all = ["when_asked", "on"])]
    pub at: Option<String>,
    /// Run each time this happens on GitHub in the project's repositories:
    /// issue-opened, issue-labeled (with --label), pr-opened, pr-merged,
    /// release (published) or checks-failed (on the default branch, or
    /// --branch).
    #[arg(long, value_name = "EVENT", conflicts_with = "when_asked")]
    pub on: Option<String>,
    /// With --on: only in this repository of the project's (owner/name;
    /// again, or comma-separated, for several). Default: those its agents
    /// write to.
    #[arg(long = "repo", value_name = "OWNER/NAME", value_delimiter = ',')]
    pub repos: Vec<String>,
    /// With --on: the label an issue gets (issue-labeled), or one an issue
    /// or pull request has (again, or comma-separated, for any of several).
    #[arg(long = "label", value_name = "LABEL", value_delimiter = ',')]
    pub labels: Vec<String>,
    /// With --on: the branch pull requests go into (pr-opened, pr-merged),
    /// or whose checks it watches (checks-failed; default: the default
    /// branch).
    #[arg(long = "branch", value_name = "BRANCH", value_delimiter = ',')]
    pub branches: Vec<String>,
    /// With --on: whose issues or pull requests start runs: anyone,
    /// writers (people who may write to the repository, and fctl's agents)
    /// or agents (fctl's). Default: anyone, but writers for pr-opened.
    #[arg(long, value_name = "WHO")]
    pub by: Option<String>,
    /// Run only when someone runs it, or another tool does with an access
    /// token (fctl schedules run).
    #[arg(long)]
    pub when_asked: bool,
    /// The time zone of the cron expression's times (an IANA name, like
    /// Europe/Stockholm). Default: this machine's, else UTC.
    #[arg(long, visible_alias = "timezone")]
    pub tz: Option<String>,
}

impl WhenArgs {
    /// Whether it narrows down an event: --repo, --label, --branch or --by.
    fn narrows(&self) -> bool {
        !self.repos.is_empty()
            || !self.labels.is_empty()
            || !self.branches.is_empty()
            || self.by.is_some()
    }

    /// The event it asks for: `--on` with what narrows it down, or what
    /// narrows down `current`'s anew, which keeps the rest.
    fn event(&self, current: Option<&ScheduleEvent>) -> Result<Option<ScheduleEvent>> {
        let base = match (&self.on, current) {
            (Some(on), _) => ScheduleEvent {
                kind: event_kind(on)?,
                repos: Vec::new(),
                labels: Vec::new(),
                branches: Vec::new(),
                by: None,
            },
            (None, Some(e)) if self.narrows() => e.clone(),
            (None, _) if self.narrows() => bail!(
                "--repo, --label, --branch and --by narrow down an event: pass --on too (fctl schedules add --help)."
            ),
            (None, _) => return Ok(None),
        };
        let given = |v: &[String], kept: Vec<String>| {
            if v.is_empty() { kept } else { v.to_vec() }
        };
        Ok(Some(ScheduleEvent {
            repos: given(&self.repos, base.repos),
            labels: given(&self.labels, base.labels),
            branches: given(&self.branches, base.branches),
            by: match &self.by {
                Some(b) => Some(event_by(b)?),
                None => base.by,
            },
            kind: base.kind,
        }))
    }
}

/// An event's kind, as `--on` names it, or as the API does.
fn event_kind(s: &str) -> Result<ScheduleEventKind> {
    let wanted = s.trim().to_ascii_lowercase().replace('-', "_");
    let kind = match wanted.as_str() {
        "issue" | "issue_opened" | "issues" => ScheduleEventKind::IssueOpened,
        "label" | "labeled" | "issue_labeled" | "issue_labelled" => ScheduleEventKind::IssueLabeled,
        "pr_opened" | "pull_request_opened" => ScheduleEventKind::PullRequestOpened,
        "pr_merged" | "merged" | "merge" | "pull_request_merged" => {
            ScheduleEventKind::PullRequestMerged
        }
        "release" | "release_published" => ScheduleEventKind::ReleasePublished,
        "checks_failed" | "checks" | "ci_failed" => ScheduleEventKind::ChecksFailed,
        _ => bail!(
            "{s:?} isn't an event fctl waits for: use issue-opened, issue-labeled, pr-opened, pr-merged, release or checks-failed."
        ),
    };
    Ok(kind)
}

fn event_by(s: &str) -> Result<ScheduleEventBy> {
    ScheduleEventBy::parse(s.trim().to_ascii_lowercase().as_str())
        .context("Use anyone, writers or agents for --by")
}

/// What a schedule's sessions start with.
#[derive(Args, Default)]
pub struct SessionArgs {
    /// The model its sessions run on (opus, sonnet, haiku, fable or a
    /// model's ID); `default` for the project's.
    #[arg(long)]
    pub model: Option<String>,
    /// Their effort level (low, medium, high, xhigh, max); `default` for
    /// the project's.
    #[arg(long)]
    pub effort: Option<String>,
    /// What their agents may do on GitHub (push, comment, triage, read);
    /// `default` for as much as the project allows.
    #[arg(long)]
    pub github: Option<String>,
    /// The branch they start from; `default` for the project's.
    #[arg(long, value_name = "BRANCH")]
    pub base: Option<String>,
}

#[derive(Subcommand)]
pub enum SchedulesCmd {
    /// The project's schedules (the default).
    #[command(visible_alias = "ls")]
    List,
    /// Set up a schedule, on your Claude subscription.
    #[command(after_help = "Examples:
  fctl schedules add \"Weekly dependencies\" --cron \"0 9 * * 1\" --tz Europe/Stockholm \\
    -m \"Update the dependencies that have new patch or minor releases, run the tests and open a pull request.\"
  fctl schedules add --cron \"0 8 * * 1-5\" --task-file triage.md
  fctl schedules add \"Release notes\" --at 2026-10-30T16:00+01:00 -m \"Draft the release notes for v1.0.\"
  fctl schedules add --at 2h -m \"Check whether the deploy went out, and tell me.\"
  fctl schedules add \"Triage new issues\" --on issue-opened --github triage \\
    -m \"Triage the issue: label it, and ask for what is missing to act on it.\"
  fctl schedules add \"Work on labeled issues\" --on issue-labeled --label fctl \\
    -m \"Do what the issue asks, and open a pull request that fixes it.\"
  fctl schedules add \"Learn from merges\" --on pr-merged --branch main --hide-sessions \\
    -m \"Read what people said in the pull request's review, and propose changes to AGENTS.md.\"
  fctl schedules add \"Fix main\" --on checks-failed -m \"Find what broke main, and open a fix.\"
  fctl schedules add \"Triage an issue\" --when-asked --github triage -m \"Triage the issue below.\"")]
    Add {
        /// Its name; default: the task's first line.
        name: Option<String>,
        /// The task each run's agent gets (or pipe it in).
        #[arg(short = 'm', long = "task", conflicts_with = "task_file")]
        task: Option<String>,
        /// Read the task from a file (`-`: stdin).
        #[arg(long, value_name = "FILE")]
        task_file: Option<PathBuf>,
        #[command(flatten)]
        when: WhenArgs,
        #[command(flatten)]
        session: SessionArgs,
        /// Set it up paused: it runs only once resumed.
        #[arg(long)]
        paused: bool,
        /// Keep its sessions off session lists (the web app's and fctl
        /// ls): its runs list them.
        #[arg(long)]
        hide_sessions: bool,
    },
    /// A schedule, its task and its latest runs.
    Show {
        /// The schedule: its id (sch_…) or its name.
        schedule: String,
    },
    /// Change a schedule. Changing someone else's (as one of the project's
    /// maintainers or owners) runs it on your Claude subscription.
    #[command(after_help = "Examples:
  fctl schedules edit \"Weekly dependencies\" --cron \"0 7 * * 1\"
  fctl schedules edit \"Triage new issues\" --on issue-opened --label bug   # bug reports alone
  fctl schedules edit \"Learn from merges\" --by agents     # pull requests fctl's agents opened
  fctl schedules edit sch_01JC… --task-file triage.md --effort high
  fctl schedules edit sch_01JC… --mine        # run it on your subscription")]
    Edit {
        /// The schedule: its id (sch_…) or its name.
        schedule: String,
        /// A new name.
        #[arg(long)]
        name: Option<String>,
        /// A new task.
        #[arg(short = 'm', long = "task", conflicts_with = "task_file")]
        task: Option<String>,
        /// Read the new task from a file (`-`: stdin).
        #[arg(long, value_name = "FILE")]
        task_file: Option<PathBuf>,
        #[command(flatten)]
        when: WhenArgs,
        #[command(flatten)]
        session: SessionArgs,
        /// Run it on your Claude subscription from now on.
        #[arg(long)]
        mine: bool,
        /// Keep its sessions, those it started already too, off session
        /// lists (the web app's and fctl ls): its runs list them.
        #[arg(long, conflicts_with = "show_sessions")]
        hide_sessions: bool,
        /// List its sessions again.
        #[arg(long)]
        show_sessions: bool,
    },
    /// Pause a schedule: it starts nothing until it is resumed.
    Pause {
        /// The schedule: its id (sch_…) or its name.
        schedule: String,
    },
    /// Resume a paused schedule (someone else's runs on your Claude
    /// subscription then).
    Resume {
        /// The schedule: its id (sch_…) or its name.
        schedule: String,
    },
    /// Run a schedule's task now, in a session on your Claude
    /// subscription. Other tools do this with an access token that runs
    /// sessions, sending what they know of why it runs.
    #[command(
        after_help = "What --context says comes after the task. From an access token, the agent reads it as information, not instructions.

Examples:
  fctl schedules run \"Triage new issues\"
  fctl schedules run sch_01JC… --context \"Issue #41 was just opened: The login page hangs\"
  gh issue view 41 | FCTL_TOKEN=fctl_pat_… fctl schedules run -p web \"Triage new issues\" --context-file -"
    )]
    Run {
        /// The schedule: its id (sch_…) or its name.
        schedule: String,
        /// What to add for this run.
        #[arg(long, conflicts_with = "context_file")]
        context: Option<String>,
        /// Read what to add from a file (`-`: stdin).
        #[arg(long, value_name = "FILE")]
        context_file: Option<PathBuf>,
    },
    /// A schedule's runs, newest first: the sessions it started, and why it
    /// started none when it didn't.
    Runs {
        /// The schedule: its id (sch_…) or its name.
        schedule: String,
        /// How many.
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// Delete a schedule. The sessions it started stay.
    #[command(visible_alias = "delete")]
    Rm {
        /// The schedule: its id (sch_…) or its name.
        schedule: String,
    },
}

pub async fn run(server: Option<&str>, args: SchedulesArgs, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let project = || super::sessions::project_or_default(args.project.clone());
    match args.cmd.unwrap_or(SchedulesCmd::List) {
        SchedulesCmd::List => {
            let project = project()?;
            let list = api.schedules(&project).await?;
            if json {
                return style::print_json(&list);
            }
            if list.schedules.is_empty() {
                println!(
                    "No schedules in {project}. Set one up: fctl schedules add NAME --cron \"0 9 * * 1\" -m \"…\""
                );
                return Ok(());
            }
            for (i, s) in list.schedules.iter().enumerate() {
                if i > 0 {
                    println!();
                }
                print_schedule(s);
            }
            Ok(())
        }
        SchedulesCmd::Add {
            name,
            task,
            task_file,
            when,
            session,
            paused,
            hide_sessions,
        } => {
            let project = project()?;
            let prompt = match (task, task_file) {
                (Some(t), _) => t,
                (None, Some(path)) => read(&path)?,
                (None, None) if !std::io::stdin().is_terminal() => read(Path::new("-"))?,
                (None, None) => {
                    bail!("What should its agents do? Pass -m \"…\" or --task-file FILE.")
                }
            };
            let (cron, at) = timing(&when)?;
            let event = when.event(None)?;
            if cron.is_none() && at.is_none() && event.is_none() && !when.when_asked {
                bail!(
                    "When does it run? Pass --cron \"0 9 * * 1\" to run again and again, --at 2h to run once, --on issue-opened (or another event on GitHub) to run each time it happens, or --when-asked."
                );
            }
            let req = CreateScheduleRequest {
                name,
                prompt,
                cron,
                at,
                event,
                timezone: Some(when.tz.clone().unwrap_or_else(local_zone)),
                model: chosen(session.model),
                effort: chosen(session.effort),
                base_branch: chosen(session.base),
                github_access: match chosen(session.github) {
                    Some(g) => Some(access(&g)?),
                    None => None,
                },
                hide_sessions: hide_sessions.then_some(true),
                enabled: paused.then_some(false),
            };
            let s = api.create_schedule(&project, &req).await?;
            if json {
                return style::print_json(&s);
            }
            println!(
                "{} Set up {} in {project}",
                style::accent("✓"),
                style::bold(&style::terminal_safe(&s.name))
            );
            println!("  {}", style::dim(&when_line(&s)));
            let on = if s.cron.is_none() && s.at.is_none() && s.event.is_none() {
                "Each run starts on the Claude subscription of whoever runs it"
            } else {
                "Its runs start on your Claude subscription"
            };
            if let Some(problem) = &s.problem {
                println!("  {}", style::amber(&style::terminal_safe(problem)));
            }
            println!(
                "  {}",
                style::dim(&format!("{on}. Run it now: fctl schedules run {}", s.id))
            );
            Ok(())
        }
        SchedulesCmd::Show { schedule } => {
            let s = find(&api, args.project.clone(), &schedule).await?;
            let runs = api.schedule_runs(s.id.as_str(), None, 5).await?;
            if json {
                return style::print_json(&s);
            }
            print_schedule(&s);
            println!();
            println!("{}", style::dim("Task"));
            for line in style::terminal_safe(&s.prompt).lines() {
                println!("  {line}");
            }
            let row = |k: &str, v: String| {
                println!("  {:<14}{}", style::dim(k), style::terminal_safe(&v))
            };
            println!();
            row(
                "Model",
                s.model.clone().unwrap_or_else(|| "the project's".into()),
            );
            row(
                "Effort",
                s.effort.clone().unwrap_or_else(|| "the project's".into()),
            );
            row(
                "On GitHub",
                s.github_access
                    .map(|g| g.as_str().to_owned())
                    .unwrap_or_else(|| "as much as the project allows".into()),
            );
            row(
                "Sessions",
                if s.hide_sessions {
                    "kept off session lists".into()
                } else {
                    "listed".into()
                },
            );
            row(
                "From branch",
                s.base_branch
                    .clone()
                    .unwrap_or_else(|| "the project's default".into()),
            );
            if !runs.runs.is_empty() {
                println!();
                println!("{}", style::dim("Latest runs"));
                for r in &runs.runs {
                    print_run(r, "  ");
                }
            }
            Ok(())
        }
        SchedulesCmd::Edit {
            schedule,
            name,
            task,
            task_file,
            when,
            session,
            mine,
            hide_sessions,
            show_sessions,
        } => {
            let s = find(&api, args.project.clone(), &schedule).await?;
            let prompt = match (task, task_file) {
                (Some(t), _) => Some(t),
                (None, Some(path)) => Some(read(&path)?),
                (None, None) => None,
            };
            let (cron, at) = timing(&when)?;
            let event = when.event(s.event.as_ref())?;
            let req = UpdateScheduleRequest {
                name,
                prompt,
                // No time: only when asked.
                cron: if when.when_asked {
                    Some(String::new())
                } else {
                    cron
                },
                at,
                event,
                timezone: when.tz,
                model: blank(session.model),
                effort: blank(session.effort),
                base_branch: blank(session.base),
                github_access: match blank(session.github) {
                    Some(g) if g.is_empty() => Some(g),
                    Some(g) => Some(access(&g)?.as_str().to_owned()),
                    None => None,
                },
                hide_sessions: if hide_sessions {
                    Some(true)
                } else {
                    show_sessions.then_some(false)
                },
                enabled: None,
                run_as_me: mine.then_some(true),
            };
            if req == UpdateScheduleRequest::default() {
                bail!(
                    "Nothing to change: pass what changes, like --cron, --on or --task (fctl schedules edit --help)."
                );
            }
            let s = api.update_schedule(s.id.as_str(), &req).await?;
            if json {
                return style::print_json(&s);
            }
            println!(
                "{} Saved {}",
                style::accent("✓"),
                style::bold(&style::terminal_safe(&s.name))
            );
            println!("  {}", style::dim(&when_line(&s)));
            Ok(())
        }
        SchedulesCmd::Pause { schedule } => {
            switch(&api, args.project.clone(), &schedule, false, json).await
        }
        SchedulesCmd::Resume { schedule } => {
            switch(&api, args.project.clone(), &schedule, true, json).await
        }
        SchedulesCmd::Run {
            schedule,
            context,
            context_file,
        } => {
            let s = find(&api, args.project.clone(), &schedule).await?;
            let context = match (context, context_file) {
                (Some(c), _) => Some(c),
                (None, Some(path)) => Some(read(&path)?),
                (None, None) => None,
            };
            let run = api
                .run_schedule(s.id.as_str(), &RunScheduleRequest { context })
                .await?;
            if json {
                return style::print_json(&run);
            }
            match &run.session {
                Some(session) => {
                    println!(
                        "{} Started session #{} from {}",
                        style::accent("✓"),
                        session.number,
                        style::bold(&style::terminal_safe(&s.name))
                    );
                    println!(
                        "  {}",
                        style::dim(&format!(
                            "It runs on your Claude subscription. Follow it: fctl attach {}",
                            session.id
                        ))
                    );
                }
                None => println!(
                    "{}",
                    style::amber(&style::terminal_safe(
                        run.reason.as_deref().unwrap_or("No session started.")
                    ))
                ),
            }
            Ok(())
        }
        SchedulesCmd::Runs { schedule, limit } => {
            let s = find(&api, args.project.clone(), &schedule).await?;
            let runs = api.schedule_runs(s.id.as_str(), None, limit).await?;
            if json {
                return style::print_json(&runs);
            }
            if runs.runs.is_empty() {
                println!(
                    "{} hasn't run yet. {}",
                    style::terminal_safe(&s.name),
                    when_line(&s)
                );
                return Ok(());
            }
            for r in &runs.runs {
                print_run(r, "");
            }
            Ok(())
        }
        SchedulesCmd::Rm { schedule } => {
            let s = find(&api, args.project.clone(), &schedule).await?;
            api.delete_schedule(s.id.as_str()).await?;
            if json {
                return style::print_json(&serde_json::json!({ "ok": true, "id": s.id }));
            }
            println!(
                "{} Deleted {}. The sessions it started stay.",
                style::accent("✓"),
                style::bold(&style::terminal_safe(&s.name))
            );
            Ok(())
        }
    }
}

/// Pauses (`on` false) or resumes a schedule.
async fn switch(
    api: &Api,
    project: Option<String>,
    schedule: &str,
    on: bool,
    json: bool,
) -> Result<()> {
    let s = find(api, project, schedule).await?;
    let s = api
        .update_schedule(
            s.id.as_str(),
            &UpdateScheduleRequest {
                enabled: Some(on),
                ..Default::default()
            },
        )
        .await?;
    if json {
        return style::print_json(&s);
    }
    println!(
        "{} {} {}",
        style::accent("✓"),
        if on { "Resumed" } else { "Paused" },
        style::bold(&style::terminal_safe(&s.name))
    );
    println!("  {}", style::dim(&when_line(&s)));
    Ok(())
}

/// A schedule by id, or by name in the project.
async fn find(api: &Api, project: Option<String>, schedule: &str) -> Result<Schedule> {
    if fctl_proto::ScheduleId::is_valid(schedule) {
        return Ok(api.get_schedule(schedule).await?);
    }
    let project = super::sessions::project_or_default(project)?;
    let list = api.schedules(&project).await?;
    let wanted = schedule.trim();
    let mut found: Vec<Schedule> = list
        .schedules
        .into_iter()
        .filter(|s| s.name.trim().eq_ignore_ascii_case(wanted))
        .collect();
    match found.len() {
        0 => bail!("No schedule in {project} is called {wanted:?}: `fctl schedules` lists them."),
        1 => Ok(found.remove(0)),
        n => bail!(
            "{n} schedules in {project} are called {wanted:?}: pass one's id (fctl schedules)."
        ),
    }
}

/// The cron expression or the time a schedule runs at, as the API takes
/// them.
fn timing(when: &WhenArgs) -> Result<(Option<String>, Option<i64>)> {
    if let Some(c) = &when.cron {
        Cron::parse(c).map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    let at = when.at.as_deref().map(at).transpose()?;
    Ok((when.cron.clone(), at))
}

/// A time to run once: `2h`, `in 30m`, `1d`, or RFC 3339 with an offset.
fn at(input: &str) -> Result<i64> {
    let t = input.trim();
    let relative = t.strip_prefix("in ").unwrap_or(t).trim();
    if let Some(ms) = duration_ms(relative) {
        return Ok(style::now_ms() + ms);
    }
    // RFC 3339, its seconds optional.
    let with_seconds = t.split_once(['T', 't', ' ']).and_then(|(date, rest)| {
        let i = rest.find(['Z', 'z', '+', '-'])?;
        let (clock, offset) = rest.split_at(i);
        Some(if clock.matches(':').count() == 1 {
            format!("{date}T{clock}:00{offset}")
        } else {
            format!("{date}T{rest}")
        })
    });
    with_seconds
        .as_deref()
        .and_then(fctl_domain::pulls::github_time)
        .with_context(|| {
            format!(
                "{t:?} isn't a time fctl reads: give how far ahead (2h, 30m, 1d) or a time with its UTC offset, like 2026-10-30T14:00+01:00"
            )
        })
}

/// `90m`, `2h`, `1d`, `1h30m`.
pub(crate) fn duration_ms(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let mut total = 0i64;
    let mut digits = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let n: i64 = digits.parse().ok()?;
        digits.clear();
        total += n * match c {
            'm' => 60_000,
            'h' => 3_600_000,
            'd' => 86_400_000,
            _ => return None,
        };
    }
    digits.is_empty().then_some(total)
}

/// This machine's time zone: `TZ`, or the zone `/etc/localtime` links to,
/// else UTC.
fn local_zone() -> String {
    let from_tz = std::env::var("TZ")
        .ok()
        .map(|t| t.trim_start_matches(':').to_owned())
        .filter(|t| t.contains('/') || t == "UTC");
    let from_link = || {
        let target = std::fs::read_link("/etc/localtime").ok()?;
        let path = target.to_string_lossy().into_owned();
        let (_, zone) = path.split_once("zoneinfo/")?;
        Some(zone.to_owned())
    };
    from_tz
        .or_else(from_link)
        .filter(|z| fctl_domain::schedules::timezone(Some(z)).is_ok())
        .unwrap_or_else(|| "UTC".into())
}

/// A setting given at creation: `default` (or nothing) for the project's.
fn chosen(v: Option<String>) -> Option<String> {
    v.filter(|v| !v.trim().eq_ignore_ascii_case("default") && !v.trim().is_empty())
}

/// A setting given in a change: `default` becomes `""`, the project's.
fn blank(v: Option<String>) -> Option<String> {
    v.map(|v| {
        if v.trim().eq_ignore_ascii_case("default") {
            String::new()
        } else {
            v
        }
    })
}

fn access(v: &str) -> Result<GithubAccess> {
    GithubAccess::parse(v.trim())
        .context("Use push, comment, triage or read, or default for the project's")
}

/// A file's text, or stdin's for `-`.
fn read(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        return Ok(s);
    }
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

/// When it runs, and next: `every Monday at 09:00 (Europe/Stockholm) · next
/// in 2d 21h`.
fn when_line(s: &Schedule) -> String {
    let when = match (&s.cron, s.at) {
        _ if s.event.is_some() => format!(
            "Runs {}",
            s.event
                .as_ref()
                .map(fctl_domain::triggers::describe)
                .unwrap_or_default()
        ),
        (Some(c), _) => {
            let words = Cron::parse(c)
                .map(|c| c.describe())
                .unwrap_or_else(|_| c.clone());
            format!("Runs {words} ({})", s.timezone)
        }
        (None, Some(at)) if s.state == ScheduleState::Done => {
            format!("Ran once, {}", style::ago(at))
        }
        // It may wait for its owner's plan, past its time.
        (None, Some(at)) => format!("Runs once, {}", style::until(s.next_run_at.unwrap_or(at))),
        (None, None) => "Runs when asked: fctl schedules run, or another tool with a token".into(),
    };
    let next = match s.state {
        ScheduleState::Paused if s.paused_reason.is_some() => "paused by fctl".into(),
        ScheduleState::Paused => "paused".into(),
        ScheduleState::Done => String::new(),
        ScheduleState::Active => s
            .next_run_at
            .filter(|_| s.cron.is_some())
            .map(|n| format!("next {}", style::until(n)))
            .unwrap_or_default(),
    };
    [when, next]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// What a schedule's agents may do on GitHub, when it holds them to less
/// than pushing, as the web app's rows say it.
fn held_to(access: Option<GithubAccess>) -> Option<&'static str> {
    match access? {
        GithubAccess::Push => None,
        GithubAccess::Comment => Some("comments only"),
        GithubAccess::Triage => Some("triage only: labels and comments"),
        GithubAccess::Read => Some("read only"),
    }
}

fn print_schedule(s: &Schedule) {
    let (dot, state) = match s.state {
        ScheduleState::Active => (style::accent("●"), String::new()),
        ScheduleState::Paused => (style::dim("○"), style::dim(" (paused)")),
        ScheduleState::Done => (style::dim("✓"), style::dim(" (done)")),
    };
    println!(
        "{dot} {}{state}  {}",
        style::bold(&style::terminal_safe(&s.name)),
        style::dim(s.id.as_str())
    );
    let mut facts = vec![when_line(s)];
    facts.extend(held_to(s.github_access).map(str::to_owned));
    if s.hide_sessions {
        facts.push("sessions kept off lists".into());
    }
    match &s.owner {
        // Each run is on the subscription of whoever runs it.
        Some(owner) if s.cron.is_none() && s.at.is_none() && s.event.is_none() => {
            facts.push(format!("set up by {}", owner.name))
        }
        Some(owner) => facts.push(format!("on {}'s Claude subscription", owner.name)),
        None => {}
    }
    println!(
        "  {}",
        style::dim(&style::terminal_safe(&facts.join(" · ")))
    );
    if let Some(why) = s.paused_reason.as_ref().or(s.problem.as_ref()) {
        println!("  {}", style::amber(&style::terminal_safe(why)));
    }
    if let Some(r) = &s.last_run {
        print_run(r, "  last run ");
    }
}

/// Who started a run: "on time", "by Ada", "by spaghetti's autopilot, for
/// Ada", "by a tool with Ada's token", "as @octocat opened issue
/// acme/web#41".
fn run_by(r: &ScheduleRun) -> String {
    match (r.trigger, &r.by, &r.autopilot) {
        (ScheduleTrigger::Time, _, _) => "on time".to_owned(),
        (ScheduleTrigger::Event, _, _) => r
            .event
            .as_ref()
            .map(|e| format!("as {}", event_line(e)))
            .unwrap_or_else(|| "on an event".to_owned()),
        (_, Some(u), Some(a)) => format!("by {}'s autopilot, for {}", a.project_slug, u.name),
        (ScheduleTrigger::Person, Some(u), None) => format!("by {}", u.name),
        (ScheduleTrigger::Token, Some(u), None) => format!("by a tool with {}'s token", u.name),
        (ScheduleTrigger::Other, Some(u), None) => format!("for {}", u.name),
        (_, None, _) => "by someone".to_owned(),
    }
}

/// What happened on GitHub that started a run: "@octocat opened issue
/// acme/web#41", "@grace merged acme/web#12 into main", "checks failed on
/// main of acme/web".
fn event_line(e: &ScheduleRunEvent) -> String {
    let repo = e.repo.full_name();
    let n = e.number.unwrap_or_default();
    let who = e
        .actor
        .as_deref()
        .map_or_else(|| "someone".to_owned(), |a| format!("@{a}"));
    match e.kind {
        ScheduleEventKind::IssueOpened => format!("{who} opened issue {repo}#{n}"),
        ScheduleEventKind::IssueLabeled => format!(
            "{who} added the label {} to issue {repo}#{n}",
            e.label.as_deref().unwrap_or_default()
        ),
        ScheduleEventKind::PullRequestOpened => format!("{who} opened pull request {repo}#{n}"),
        ScheduleEventKind::PullRequestMerged => format!(
            "{who} merged pull request {repo}#{n}{}",
            e.branch
                .as_deref()
                .map(|b| format!(" into {b}"))
                .unwrap_or_default()
        ),
        ScheduleEventKind::ReleasePublished => format!(
            "{who} published release {} of {repo}",
            e.tag.as_deref().unwrap_or_default()
        ),
        ScheduleEventKind::ChecksFailed => format!(
            "checks failed on {} of {repo}",
            e.branch.as_deref().unwrap_or_default()
        ),
        ScheduleEventKind::Other => format!("something happened in {repo}"),
    }
}

/// A run: what came of it, its session, who started it and when.
fn print_run(r: &ScheduleRun, lead: &str) {
    let what = match (r.outcome, &r.session) {
        (ScheduleRunOutcome::Started, Some(session)) => format!(
            "{} session #{} {}",
            style::accent("started"),
            session.number,
            style::terminal_safe(&session.title)
        ),
        (ScheduleRunOutcome::Started, None) => style::accent("started"),
        (ScheduleRunOutcome::Skipped, _) => style::amber("skipped"),
        (ScheduleRunOutcome::Failed, _) => style::red("failed"),
    };
    let by = run_by(r);
    println!(
        "{lead}{what} {}",
        style::dim(&style::terminal_safe(&format!(
            "· {by} · {}",
            style::ago(r.at)
        )))
    );
    if let Some(reason) = &r.reason {
        let indent = lead.len() - lead.trim_start().len() + 2;
        println!(
            "{}{}",
            " ".repeat(indent),
            style::dim(&style::terminal_safe(reason))
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_who_started_a_run() -> anyhow::Result<()> {
        let run = |trigger: &str, autopilot: bool| -> anyhow::Result<ScheduleRun> {
            let mut v = serde_json::json!({
                "id": "shr_1", "schedule_id": "sch_1", "due_at": 1, "at": 1,
                "outcome": "started", "trigger": trigger,
                "by": { "id": "usr_1", "name": "Carl Oscar Aaro" }
            });
            if autopilot {
                v["autopilot"] =
                    serde_json::json!({ "project_id": "prj_1", "project_slug": "spaghetti" });
            }
            Ok(serde_json::from_value(v)?)
        };
        assert_eq!(run_by(&run("time", false)?), "on time");
        assert_eq!(run_by(&run("person", false)?), "by Carl Oscar Aaro");
        assert_eq!(
            run_by(&run("token", false)?),
            "by a tool with Carl Oscar Aaro's token"
        );
        assert_eq!(
            run_by(&run("token", true)?),
            "by spaghetti's autopilot, for Carl Oscar Aaro"
        );
        let mut on_event = run("event", false)?;
        on_event.event = Some(serde_json::from_value(serde_json::json!({
            "kind": "issue_opened", "repo": { "owner": "acme", "name": "web" }, "number": 41,
            "url": "https://github.com/acme/web/issues/41", "actor": "octocat"
        }))?);
        assert_eq!(run_by(&on_event), "as @octocat opened issue acme/web#41");
        // A trigger a newer control plane knows reads.
        let newer = run("webhook", false)?;
        assert_eq!(newer.trigger, ScheduleTrigger::Other);
        Ok(())
    }

    #[test]
    fn reads_the_events_it_waits_for() -> anyhow::Result<()> {
        assert_eq!(event_kind("issue-opened")?, ScheduleEventKind::IssueOpened);
        assert_eq!(
            event_kind("PR-merged")?,
            ScheduleEventKind::PullRequestMerged
        );
        assert_eq!(
            event_kind("pull_request_opened")?,
            ScheduleEventKind::PullRequestOpened
        );
        assert_eq!(
            event_kind("checks-failed")?,
            ScheduleEventKind::ChecksFailed
        );
        assert!(event_kind("push").is_err());
        let when = WhenArgs {
            on: Some("issue-labeled".into()),
            labels: vec!["agent".into()],
            ..Default::default()
        };
        let e = when.event(None)?;
        assert_eq!(
            e.as_ref().map(|e| (e.kind, e.labels.clone())),
            Some((ScheduleEventKind::IssueLabeled, vec!["agent".to_owned()]))
        );
        // Narrowing down what a schedule waits for keeps the rest.
        let narrower = WhenArgs {
            branches: vec!["main".into()],
            ..Default::default()
        };
        let current = ScheduleEvent {
            kind: ScheduleEventKind::PullRequestMerged,
            repos: vec!["acme/web".into()],
            labels: Vec::new(),
            branches: Vec::new(),
            by: Some(ScheduleEventBy::Agents),
        };
        let changed = narrower.event(Some(&current))?;
        assert_eq!(
            changed.map(|e| (e.repos, e.branches, e.by)),
            Some((
                vec!["acme/web".to_owned()],
                vec!["main".to_owned()],
                Some(ScheduleEventBy::Agents)
            ))
        );
        assert!(narrower.event(None).is_err());
        assert_eq!(WhenArgs::default().event(Some(&current))?, None);
        Ok(())
    }

    #[test]
    fn says_what_a_schedules_agents_may_do_on_github() {
        assert_eq!(held_to(None), None);
        assert_eq!(held_to(Some(GithubAccess::Push)), None);
        assert_eq!(
            held_to(Some(GithubAccess::Triage)),
            Some("triage only: labels and comments")
        );
        assert_eq!(held_to(Some(GithubAccess::Read)), Some("read only"));
    }

    #[test]
    fn reads_how_far_ahead() {
        assert_eq!(duration_ms("90m"), Some(90 * 60_000));
        assert_eq!(duration_ms("1h30m"), Some(90 * 60_000));
        assert_eq!(duration_ms("2d"), Some(2 * 86_400_000));
        assert_eq!(duration_ms("2"), None);
        assert_eq!(duration_ms("2w"), None);
        assert_eq!(duration_ms(""), None);
        let soon = at("in 2h").ok();
        assert!(soon.is_some_and(|t| t > style::now_ms() + 7_100_000));
        assert_eq!(
            at("2026-10-30T14:00:00+01:00").ok(),
            Some(1_793_365_200_000)
        );
        assert!(at("next tuesday").is_err());
    }

    #[test]
    fn settings_say_default_for_the_projects() {
        assert_eq!(chosen(Some("default".into())), None);
        assert_eq!(chosen(Some("opus".into())), Some("opus".into()));
        assert_eq!(blank(Some("Default".into())), Some(String::new()));
        assert_eq!(blank(Some("high".into())), Some("high".into()));
        assert_eq!(blank(None), None);
    }
}
