//! `fctl autopilot` (experimental): a project's autopilot, an agent that
//! runs the project, or its whole workspace, on its own.
//!
//! People put a project on autopilot, steer it with notes, answer what it
//! asks (secrets too, which go into the project's secrets), wake, pause,
//! resume and take it off. In its own session's sandbox, the autopilot
//! reads where things stand (`brief`), keeps its plan and log, asks people
//! and says when to wake: its `fctl` reaches the API through fctl-agent
//! (`config::api`), with `FCTL_AUTOPILOT` naming its project.

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::builder::BoolishValueParser;
use clap::{Args, Subcommand};
use fctl_proto::agent::AUTOPILOT_ENV;
use fctl_proto::autopilot::*;
use fctl_proto::session::SessionStatus;
use std::io::{IsTerminal, Read};
use std::path::PathBuf;

#[derive(Args)]
pub struct AutopilotArgs {
    /// The project (slug or id): default, the autopilot's own in its
    /// sandbox, else the one `fctl projects use` chose.
    #[arg(short, long, global = true)]
    pub project: Option<String>,
    #[command(subcommand)]
    pub cmd: Option<AutopilotCmd>,
}

/// How it works, when putting it on or changing it.
#[derive(Args, Default)]
pub struct Settings {
    /// Where it should take the project (Markdown).
    #[arg(short, long, conflicts_with = "direction_file")]
    pub direction: Option<String>,
    /// Read the direction from a file (`-`: stdin).
    #[arg(long, value_name = "FILE")]
    pub direction_file: Option<PathBuf>,
    /// How long it goes without waking, at most: 15m, 30m, 1h, 2h, 4h, 8h,
    /// 12h or 1d.
    #[arg(long, value_name = "DURATION")]
    pub check_in: Option<String>,
    /// How many of its sessions work at once, at most (1 to 8).
    #[arg(long, value_name = "N")]
    pub sessions: Option<u32>,
    /// What it may do besides starting and steering sessions, a list:
    /// merge (its sessions merge their pull requests once checks pass),
    /// settings, secrets, repos, schedules and projects (a workspace's), or
    /// all (every power its scope has) or none.
    #[arg(long, value_name = "POWERS")]
    pub powers: Option<String>,
    /// The model of its own sessions (opus, sonnet…); `default` for the
    /// project's.
    #[arg(long)]
    pub model: Option<String>,
    /// Their effort level; `default` for the project's.
    #[arg(long)]
    pub effort: Option<String>,
    /// The context size, 100k to 1m tokens, at which a fresh session of
    /// its own takes over from the one it works in, starting from its plan
    /// and log, at the first wake between turns; `never` for only after 12
    /// hours. Each wake reads its whole conversation again: a smaller one
    /// spends less of the plan.
    #[arg(long, value_name = "TOKENS|never", value_parser = handoff_arg)]
    pub handoff_at: Option<u32>,
    /// Keep it on your subscription when others change or resume it
    /// (yours to choose, as its owner): those who steer it then spend your
    /// Claude plan, as you. Only where you trust everyone who steers it,
    /// the project's maintainers and owners (a workspace's: its admins).
    #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
    pub keep_owner: Option<bool>,
}

#[derive(Subcommand)]
pub enum AutopilotCmd {
    /// Where it stands: its state, what it asks, its notes, sessions and
    /// plan (the default).
    Status,
    /// Put the project on autopilot, on your Claude subscription: its
    /// workspace's admins let projects have one first.
    #[command(after_help = "Examples:
  fctl autopilot on -p crm --direction \"Build a CRM for small agencies: contacts, deals and a pipeline board. Ship it to crm.example.com.\"
  fctl autopilot on -p crm --direction-file direction.md --check-in 30m --sessions 4
  fctl autopilot on -p ops --workspace --direction \"Keep every project's dependencies, CI and docs healthy.\"
  fctl autopilot on -p crm --powers settings,schedules   # no merging, secrets or repositories")]
    On {
        #[command(flatten)]
        settings: Settings,
        /// Put the whole workspace on autopilot, working from this project:
        /// it reaches all its projects, and creates new ones (admins).
        #[arg(long)]
        workspace: bool,
    },
    /// Change its direction, powers, check-in, sessions or model. Changing
    /// how much it works when it runs on someone else's subscription (or
    /// --mine) runs it on yours, unless they keep it (--keep-owner).
    #[command(after_help = "Examples:
  fctl autopilot set -p crm --check-in 2h --sessions 4
  fctl autopilot set -p crm --handoff-at 250k   # a fresh session takes over once its context holds 250K tokens
  fctl autopilot set -p crm --keep-owner on   # yours: keep it on your plan when others change it
  fctl autopilot set -p crm --mine            # run it on your subscription from now on")]
    Set {
        #[command(flatten)]
        settings: Settings,
        /// Run it on your Claude subscription from now on.
        #[arg(long)]
        mine: bool,
    },
    /// Take the project off autopilot. It keeps its direction, notes, plan,
    /// log and open asks for when it's put on again.
    Off {
        /// Stop the sessions it started that still work, too.
        #[arg(long)]
        stop_sessions: bool,
    },
    /// Pause it: it wakes no more until resumed. In its sandbox, the
    /// autopilot pauses itself once its direction is done.
    Pause {
        /// Why.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Resume it: it wakes at once, on your subscription from then on
    /// when it ran on someone else's, unless they keep it.
    Resume,
    /// Wake it now. In its sandbox, the autopilot says when to wake next,
    /// unless something wakes it sooner.
    #[command(after_help = "Examples:
  fctl autopilot wake -p crm             # now
  fctl autopilot wake --in 30m           # the autopilot, in its sandbox: 5m to 1d")]
    Wake {
        /// In how long (5m to 1d): the autopilot, in its sandbox.
        #[arg(long = "in", value_name = "DURATION")]
        after: Option<String>,
    },
    /// Steer it with a note: a priority, a constraint, context it should
    /// know. It hears at once, and keeps it until someone removes it.
    #[command(after_help = "Examples:
  fctl autopilot steer -p crm \"Focus on the pipeline board this week; leave imports for later.\"
  fctl autopilot steer -p crm \"We launch on Friday: nothing risky after Wednesday.\"")]
    Steer {
        /// The note (or pipe it in).
        note: Vec<String>,
    },
    /// Its notes, newest first, or remove one.
    Notes {
        /// Remove this note (its id, apn_…).
        #[arg(long, value_name = "NOTE")]
        remove: Option<String>,
    },
    /// What it asks people, or what every autopilot asks you (--all).
    Asks {
        /// Across your workspaces.
        #[arg(long)]
        all: bool,
    },
    /// Answer what it asked.
    Answer {
        /// What it asked (its id, apa_…).
        ask: String,
        /// The answer (or pipe it in).
        answer: Vec<String>,
    },
    /// Give it the secret it asked for: asked without showing what you
    /// type, or read from stdin. It goes into the project's secrets; the
    /// autopilot hears only that it's set.
    Secret {
        /// What it asked (its id, apa_…).
        ask: String,
    },
    /// Decline what it asked.
    Decline {
        /// What it asked (its id, apa_…).
        ask: String,
        /// Why, for the autopilot.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Its log, newest first. In its sandbox, with text (`-` to read it
    /// from stdin), the autopilot writes down what it did, decided or plans.
    #[command(after_help = "Examples:
  fctl autopilot log -p crm --limit 100
  fctl autopilot log \"Started crm#12 for the pipeline board; the import work waits for the CSV format.\"")]
    Log {
        /// What to write down (the autopilot, in its sandbox).
        text: Vec<String>,
        /// How many entries to list.
        #[arg(long, default_value_t = 30)]
        limit: u32,
    },
    /// Its plan. In its sandbox, the autopilot writes it whole, from a file
    /// or stdin.
    Plan {
        /// Write the plan from this file (`-`: stdin): the autopilot.
        #[arg(long, value_name = "FILE")]
        file: Option<PathBuf>,
    },
    /// The autopilot asks people a question, with answers to click, or for
    /// a secret, and goes on meanwhile: fctl wakes it with the answer.
    #[command(after_help = "Examples:
  fctl autopilot ask \"Which payment provider should checkout use?\" -o \"Stripe\" -o \"Adyen\"
  fctl autopilot ask \"Which of these two pipeline boards?\" --detail-file options.md -o \"A, the compact one\" -o \"B, with avatars\"
  fctl autopilot ask --secret CLOUDFLARE_API_TOKEN \"To deploy the app to Cloudflare Workers from CI\"")]
    Ask {
        /// The question, or what the secret is for, in a sentence or two.
        text: Vec<String>,
        /// An answer people can give with a click (repeatable, up to 6).
        #[arg(short, long = "option", value_name = "ANSWER")]
        options: Vec<String>,
        /// What helps them answer, in Markdown: links, and screenshots you
        /// published with fctl publish.
        #[arg(long, conflicts_with = "detail_file")]
        detail: Option<String>,
        /// Read the detail from a file.
        #[arg(long, value_name = "FILE")]
        detail_file: Option<PathBuf>,
        /// Ask for a secret by this name (an environment variable's, like
        /// STRIPE_KEY), which people type in where it asks.
        #[arg(long, value_name = "NAME", conflicts_with = "options")]
        secret: Option<String>,
        /// The project the secret goes into: default, its own.
        #[arg(long = "for", value_name = "PROJECT", requires = "secret")]
        for_project: Option<String>,
    },
    /// The autopilot takes back what it asked: it found out itself.
    Withdraw {
        /// What it asked (its id, apa_…).
        ask: String,
    },
    /// Where things stand, as Markdown: what the autopilot reads as it
    /// wakes.
    Brief,
}

pub async fn run(server: Option<&str>, args: AutopilotArgs, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let cmd = args.cmd.unwrap_or(AutopilotCmd::Status);
    if let AutopilotCmd::Asks { all: true } = cmd {
        let list = api.autopilot_asks().await?;
        if json {
            return style::print_json(&list);
        }
        if list.asks.is_empty() {
            println!("No autopilot asks you anything.");
        }
        for open in &list.asks {
            println!("{}", style::bold(&open.project_name));
            print_ask(&open.ask);
        }
        return Ok(());
    }
    let project = project(args.project)?;
    let p = project.as_str();
    match cmd {
        AutopilotCmd::Status => {
            let a = api.autopilot(p).await?;
            if json {
                return style::print_json(&a);
            }
            print_status(&a);
        }
        AutopilotCmd::On {
            settings,
            workspace,
        } => {
            let scope = if workspace {
                AutopilotScope::Workspace
            } else {
                scope_for_all(&api, p, &settings).await
            };
            let mut req = settings.request(scope)?;
            if workspace {
                req.scope = Some(AutopilotScope::Workspace);
            }
            // On again keeps the direction it had, unless given another;
            // the control plane asks for one where it had none.
            let a = api.put_autopilot(p, &req).await?;
            if json {
                return style::print_json(&a);
            }
            println!(
                "{} {} is on autopilot: it wakes now, on your Claude subscription. Follow it with fctl autopilot -p {p}.",
                style::accent("✓"),
                style::bold(&a.project_slug),
            );
        }
        AutopilotCmd::Set { settings, mine } => {
            let scope = scope_for_all(&api, p, &settings).await;
            let mut req = settings.request(scope)?;
            req.take_over = mine;
            let a = api.put_autopilot(p, &req).await?;
            if json {
                return style::print_json(&a);
            }
            println!("{} Changed its autopilot", style::accent("✓"));
            print_rules(&a);
        }
        AutopilotCmd::Off { stop_sessions } => {
            let answer = api
                .autopilot_off(p, &AutopilotOffRequest { stop_sessions })
                .await?;
            if json {
                return style::print_json(&answer);
            }
            println!(
                "{} {p} is off autopilot. Its direction, notes, plan, log and open asks stay for when it's put on again.",
                style::accent("✓")
            );
        }
        AutopilotCmd::Pause { reason } => {
            let answer = api
                .pause_autopilot(p, &PauseAutopilotRequest { reason })
                .await?;
            if json {
                return style::print_json(&answer);
            }
            if in_its_sandbox() {
                println!(
                    "{} Paused: fctl wakes you no more until someone resumes you. End your turn now.",
                    style::accent("✓")
                );
            } else {
                println!(
                    "{} Paused: it wakes no more until resumed (fctl autopilot resume -p {p}).",
                    style::accent("✓")
                );
            }
        }
        AutopilotCmd::Resume => {
            let a = api.resume_autopilot(p).await?;
            if json {
                return style::print_json(&a);
            }
            println!("{} Resumed: it wakes now", style::accent("✓"));
        }
        AutopilotCmd::Wake { after } => {
            if after.is_some() && !in_its_sandbox() {
                bail!(
                    "--in is for the autopilot, in its sandbox, to say when to wake next: people wake it now (fctl autopilot wake -p {p})"
                );
            }
            let req = match after.as_deref() {
                Some(d) => {
                    let ms = crate::cmd::schedules::duration_ms(d).with_context(|| {
                        format!("{d:?} isn't a duration fctl reads: 30m, 2h, 1d")
                    })?;
                    WakeAutopilotRequest {
                        in_minutes: Some(u32::try_from(ms / 60_000).unwrap_or(u32::MAX)),
                        at: None,
                    }
                }
                None => WakeAutopilotRequest::default(),
            };
            let answer = api.wake_autopilot(p, &req).await?;
            if json {
                return style::print_json(&answer);
            }
            // The autopilot hears when it wakes next; people, that it wakes.
            match answer
                .get("wake_at")
                .and_then(serde_json::Value::as_i64)
                .filter(|_| in_its_sandbox())
            {
                Some(at) => println!(
                    "{} fctl wakes you {}, unless something needs you sooner",
                    style::accent("✓"),
                    style::until(at)
                ),
                None => println!("{} It wakes now, for what's new", style::accent("✓")),
            }
        }
        AutopilotCmd::Steer { note } => {
            let text = words_or_stdin(note, "Write the note: fctl autopilot steer \"…\"")?;
            let a = api.add_autopilot_note(p, &text).await?;
            if json {
                return style::print_json(&a);
            }
            println!(
                "{} It hears your note now, and keeps it until someone removes it",
                style::accent("✓")
            );
        }
        AutopilotCmd::Notes { remove } => {
            let a = match remove {
                Some(note) => api.remove_autopilot_note(p, &note).await?,
                None => api.autopilot(p).await?,
            };
            if json {
                return style::print_json(&a.notes);
            }
            if a.notes.is_empty() {
                println!("No notes. Steer it with fctl autopilot steer \"…\".");
            }
            for n in &a.notes {
                print_note(n);
            }
        }
        AutopilotCmd::Asks { all: false } => {
            let a = api.autopilot(p).await?;
            if json {
                return style::print_json(&a.asks);
            }
            if a.asks.is_empty() {
                println!("It asks nothing.");
            }
            for ask in &a.asks {
                print_ask(ask);
            }
        }
        AutopilotCmd::Asks { all: true } => unreachable!("handled above"),
        AutopilotCmd::Answer { ask, answer } => {
            let text = words_or_stdin(
                answer,
                "Write the answer: fctl autopilot answer apa_… \"…\"",
            )?;
            let a = api
                .answer_ask(
                    p,
                    &ask,
                    &AnswerAutopilotAskRequest {
                        answer: Some(text),
                        secret: None,
                    },
                )
                .await?;
            if json {
                return style::print_json(&a);
            }
            println!("{} Answered: it hears now", style::accent("✓"));
        }
        AutopilotCmd::Secret { ask } => {
            let a = api.autopilot(p).await?;
            let Some(asked) = a.asks.iter().find(|x| x.id == ask) else {
                bail!("It asks nothing by that id: fctl autopilot asks -p {p} lists what it asks");
            };
            let Some(secret) = &asked.secret else {
                bail!("That ask isn't for a secret: answer it with fctl autopilot answer");
            };
            let value = crate::cmd::project_secrets::value(&secret.name)?;
            let a = api
                .answer_ask(
                    p,
                    &ask,
                    &AnswerAutopilotAskRequest {
                        answer: None,
                        secret: Some(value),
                    },
                )
                .await?;
            if json {
                return style::print_json(&a);
            }
            println!(
                "{} Set {} in {}'s secrets: the autopilot hears it's set, and sessions that start from now on have it",
                style::accent("✓"),
                style::bold(&secret.name),
                secret.project_slug
            );
        }
        AutopilotCmd::Decline { ask, reason } => {
            let a = api
                .decline_ask(p, &ask, &DeclineAutopilotAskRequest { reason })
                .await?;
            if json {
                return style::print_json(&a);
            }
            println!("{} Declined: it hears now", style::accent("✓"));
        }
        AutopilotCmd::Log { text, limit } => {
            // Text writes an entry (`-` reads it from stdin); none lists them.
            if !text.is_empty() {
                let text = if text == ["-"] {
                    words_or_stdin(Vec::new(), "Pipe in what to log: … | fctl autopilot log -")?
                } else {
                    words_or_stdin(text, "Write what to log: fctl autopilot log \"…\"")?
                };
                let answer = api.write_autopilot_log(p, &text).await?;
                if json {
                    return style::print_json(&answer);
                }
                println!("{} Logged", style::accent("✓"));
                return Ok(());
            }
            let log = api.autopilot_log(p, None, limit.clamp(1, 200)).await?;
            if json {
                return style::print_json(&log);
            }
            print_log(&log);
        }
        AutopilotCmd::Plan { file } => match file {
            Some(f) => {
                let plan = read_file_or_stdin(&f)?;
                let answer = api.set_autopilot_plan(p, &plan).await?;
                if json {
                    return style::print_json(&answer);
                }
                println!(
                    "{} {}",
                    style::accent("✓"),
                    if plan.trim().is_empty() {
                        "Cleared its plan"
                    } else {
                        "Wrote its plan: people read it on the project's Autopilot tab"
                    }
                );
            }
            None => {
                let a = api.autopilot(p).await?;
                if json {
                    return style::print_json(&serde_json::json!({
                        "plan": a.plan,
                        "plan_at": a.plan_at,
                    }));
                }
                match &a.plan {
                    Some(plan) => println!("{}", block(plan)),
                    None => println!("No plan yet."),
                }
            }
        },
        AutopilotCmd::Ask {
            text,
            options,
            detail,
            detail_file,
            secret,
            for_project,
        } => {
            let text = text.join(" ");
            if text.trim().is_empty() {
                bail!(match secret {
                    Some(_) =>
                        "Say what the secret is for: fctl autopilot ask --secret NAME \"To deploy from CI\"",
                    None =>
                        "Ask something: fctl autopilot ask \"Which payment provider?\" -o Stripe -o Adyen",
                });
            }
            let detail = match (detail, detail_file) {
                (Some(d), _) => Some(d),
                (None, Some(f)) => Some(read_file_or_stdin(&f)?),
                (None, None) => None,
            };
            let ask = api
                .ask_people(
                    p,
                    &CreateAutopilotAskRequest {
                        text,
                        detail,
                        options,
                        secret,
                        project: for_project,
                    },
                )
                .await?;
            if json {
                return style::print_json(&ask);
            }
            println!(
                "{} Asked ({}): people see it on the Autopilot tab and hear of it. Go on with what you can: fctl wakes you with the answer.",
                style::accent("✓"),
                ask.id
            );
        }
        AutopilotCmd::Withdraw { ask } => {
            let answer = api.withdraw_ask(p, &ask).await?;
            if json {
                return style::print_json(&answer);
            }
            println!("{} Took it back", style::accent("✓"));
        }
        AutopilotCmd::Brief => {
            let b = api.autopilot_brief(p).await?;
            if json {
                return style::print_json(&b);
            }
            println!("{}", block(&b.brief));
        }
    }
    Ok(())
}

/// The project: the one named, the autopilot's own in its sandbox, or the
/// default.
fn project(named: Option<String>) -> Result<String> {
    named
        .or_else(|| std::env::var(AUTOPILOT_ENV).ok().filter(|p| !p.is_empty()))
        .or_else(|| config::CliConfig::load().project)
        .context(
            "Which project? Pass --project, or set a default with `fctl projects use <project>`.",
        )
}

/// Whether this runs in an autopilot's own session's sandbox.
fn in_its_sandbox() -> bool {
    std::env::var(AUTOPILOT_ENV).is_ok_and(|p| !p.is_empty())
}

/// The scope whose every power `--powers all` asks for: that of the
/// project's autopilot, as it keeps it when put on again, else a
/// project's. Asked only for `all`.
async fn scope_for_all(
    api: &fctl_client::Api,
    project: &str,
    settings: &Settings,
) -> AutopilotScope {
    match settings.powers.as_deref().map(str::trim) {
        Some("all") => api
            .autopilot(project)
            .await
            .map_or(AutopilotScope::Project, |a| a.scope),
        _ => AutopilotScope::Project,
    }
}

impl Settings {
    /// The request, `--powers all` meaning every power of `scope`.
    fn request(self, scope: AutopilotScope) -> Result<PutAutopilotRequest> {
        let direction = match (self.direction, self.direction_file) {
            (Some(d), _) => Some(d),
            (None, Some(f)) => Some(read_file_or_stdin(&f)?),
            (None, None) => None,
        };
        let check_in_minutes = self
            .check_in
            .as_deref()
            .map(|c| {
                crate::cmd::schedules::duration_ms(c)
                    .map(|ms| u32::try_from(ms / 60_000).unwrap_or(0))
                    .with_context(|| format!("{c:?} isn't a duration fctl reads: 30m, 1h, 1d"))
            })
            .transpose()?;
        let powers = self
            .powers
            .as_deref()
            .map(|p| parse_powers(p, scope))
            .transpose()?;
        Ok(PutAutopilotRequest {
            direction,
            scope: None,
            powers,
            check_in_minutes,
            crew_max: self.sessions,
            model: self.model,
            effort: self.effort,
            handoff_at: self.handoff_at,
            take_over: false,
            keep_owner: self.keep_owner,
        })
    }
}

/// `--handoff-at`: a context size like 250k, or `never` (`0`, as the API
/// takes it).
fn handoff_arg(s: &str) -> std::result::Result<u32, String> {
    use fctl_domain::context;
    if matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "never" | "off" | "default"
    ) {
        return Ok(0);
    }
    context::parse(s)
        .and_then(|t| u32::try_from(t).ok())
        .filter(|&t| context::valid(t))
        .ok_or_else(|| {
            format!(
                "use {} to {} tokens (like 250k), or never",
                context::short(context::MIN_TOKENS).to_lowercase(),
                context::short(context::MAX_TOKENS).to_lowercase()
            )
        })
}

/// When a fresh session takes over from its own, and how full that one's
/// context is.
fn handoff_line(a: &Autopilot) -> String {
    let shift = fctl_domain::autopilot::duration(
        u32::try_from(fctl_domain::autopilot::SHIFT_MS / 60_000).unwrap_or(720),
    );
    let mut line = match a.handoff_at {
        Some(at) => format!(
            "A fresh session takes over at {} tokens of context, or after {shift}",
            fctl_domain::context::short(at)
        ),
        None => format!("A fresh session takes over after {shift}"),
    };
    if let Some(t) = a.session.as_ref().and_then(|s| s.context_tokens) {
        line.push_str(&format!(
            "; its session holds {}",
            fctl_domain::context::short(u32::try_from(t).unwrap_or(u32::MAX))
        ));
    }
    line
}

/// `settings,repos`, `all` (every power of `scope`: a project's autopilot
/// creates no projects) or `none`.
fn parse_powers(s: &str, scope: AutopilotScope) -> Result<Vec<AutopilotPower>> {
    match s.trim() {
        "all" => return Ok(fctl_domain::autopilot::default_powers(scope)),
        "none" | "" => return Ok(Vec::new()),
        _ => {}
    }
    s.split(',')
        .map(|p| {
            let p = p.trim();
            AutopilotPower::parse(match p {
                "repositories" => "repos",
                other => other,
            })
            .with_context(|| {
                format!("{p:?} isn't a power: merge, settings, secrets, repos, schedules, projects, all or none")
            })
        })
        .collect()
}

fn words_or_stdin(words: Vec<String>, missing: &str) -> Result<String> {
    let mut text = words.join(" ");
    if text.trim().is_empty() && !std::io::stdin().is_terminal() {
        std::io::stdin()
            .read_to_string(&mut text)
            .context("reading stdin")?;
    }
    if text.trim().is_empty() {
        bail!("{missing}");
    }
    Ok(text.trim().to_owned())
}

fn read_file_or_stdin(f: &std::path::Path) -> Result<String> {
    if f.as_os_str() == "-" {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("reading stdin")?;
        return Ok(text);
    }
    std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))
}

// ───────────────────────────── printing ─────────────────────────────

/// Text from the control plane, its lines as they are: Markdown the
/// autopilot reads (its brief, its plan), each line kept to what a
/// terminal shows plainly.
fn block(text: &str) -> String {
    text.lines()
        .map(style::plain)
        .collect::<Vec<_>>()
        .join("\n")
}

fn state_line(a: &Autopilot) -> String {
    match a.state {
        AutopilotState::On => {
            let at_work = a.session.as_ref().is_some_and(|s| {
                matches!(
                    s.status,
                    SessionStatus::Running
                        | SessionStatus::Preparing
                        | SessionStatus::Provisioning
                        | SessionStatus::Queued
                )
            });
            if at_work {
                let s = a.session.as_ref().map(|s| s.number).unwrap_or_default();
                style::accent(&format!("● At work in {}#{s}", a.project_slug))
            } else {
                match (a.wake_at, a.wake_reason) {
                    (Some(at), Some(AutopilotWakeReason::CheckIn) | None) => {
                        style::blue(&format!("○ On · checks in {}", style::until(at)))
                    }
                    (Some(at), Some(AutopilotWakeReason::Asked)) => {
                        style::blue(&format!("○ On · wakes {}, as it asked", style::until(at)))
                    }
                    (Some(at), Some(_)) => {
                        style::blue(&format!("○ On · wakes {} for what's new", style::until(at)))
                    }
                    (None, _) => style::blue("○ On"),
                }
            }
        }
        AutopilotState::Paused => match &a.paused_reason {
            Some(why) => style::amber(&format!("◍ Paused: {why}")),
            None => style::amber("◍ Paused"),
        },
        AutopilotState::Off => style::dim("○ Off"),
    }
}

fn print_rules(a: &Autopilot) {
    let whose = a
        .owner
        .as_ref()
        .map_or("its owner".to_owned(), |o| o.name.clone());
    println!(
        "  {}",
        style::dim(&format!(
            "On {whose}'s Claude subscription{} · up to {} at once · checks in every {}",
            if a.keep_owner {
                ", even when others change it"
            } else {
                ""
            },
            style::count(a.crew_max, "session", "sessions"),
            fctl_domain::autopilot::duration(a.check_in_minutes)
        ))
    );
    println!("  {}", style::dim(&handoff_line(a)));
    let changes: Vec<&str> = a
        .powers
        .iter()
        .filter_map(|p| match p {
            AutopilotPower::Settings => Some("settings"),
            AutopilotPower::Secrets => Some("secrets"),
            AutopilotPower::Repos => Some("repositories"),
            AutopilotPower::Schedules => Some("schedules"),
            AutopilotPower::Merge | AutopilotPower::Projects | AutopilotPower::Other => None,
        })
        .collect();
    let mut line = String::from(if a.powers.contains(&AutopilotPower::Merge) {
        "Starts and steers sessions, which merge their pull requests"
    } else {
        "Starts and steers sessions, which leave merging to people"
    });
    if let Some((last, rest)) = changes.split_last() {
        line.push_str("; changes ");
        if !rest.is_empty() {
            line.push_str(&format!("{} and ", rest.join(", ")));
        }
        line.push_str(last);
    }
    if a.powers.contains(&AutopilotPower::Projects) {
        line.push_str("; creates projects");
    }
    println!("  {}", style::dim(&line));
}

fn print_status(a: &Autopilot) {
    let scope = match a.scope {
        AutopilotScope::Project => "",
        AutopilotScope::Workspace => " · the whole workspace",
    };
    println!(
        "{} {}",
        style::bold(&format!("Autopilot of {}{scope}", a.project_slug)),
        style::dim("· experimental")
    );
    println!("  {}", state_line(a));
    if let Some(problem) = &a.problem {
        println!("  {}", style::amber(problem));
    }
    print_rules(a);
    if !a.can.read {
        println!();
        println!(
            "{}",
            style::dim(
                "It works across the workspace, in projects you may not see: its plan, sessions, asks and log are for the workspace's admins, who steer it."
            )
        );
        return;
    }
    println!();
    let by = a
        .direction_by
        .as_ref()
        .map(|u| format!("{}, ", u.name))
        .unwrap_or_default();
    println!(
        "{} {}",
        style::bold("Direction"),
        style::dim(&format!("({by}{})", style::ago(a.direction_at)))
    );
    for line in a.direction.lines() {
        println!("  {}", style::plain(line));
    }
    let open: Vec<&AutopilotAsk> = a
        .asks
        .iter()
        .filter(|x| x.status == AutopilotAskStatus::Open)
        .collect();
    if !open.is_empty() {
        println!();
        println!(
            "{} {}",
            style::bold("It asks"),
            style::dim(&format!("({})", open.len()))
        );
        for ask in open {
            print_ask(ask);
        }
    }
    if !a.notes.is_empty() {
        println!();
        println!("{}", style::bold("Notes"));
        for n in &a.notes {
            print_note(n);
        }
    }
    if !a.crew.is_empty() {
        println!();
        println!(
            "{} {}",
            style::bold("Its sessions"),
            style::dim(&format!("({} in all)", a.crew_total))
        );
        for c in &a.crew {
            let question = c
                .question
                .as_deref()
                .map(|q| style::blue(&format!(" · asks: {q}")))
                .unwrap_or_default();
            println!(
                "  {:<28} {}{}  {}",
                style::status(c.status),
                style::bold(&format!("{}#{}", c.project_slug, c.number)),
                question,
                style::plain(&c.title)
            );
        }
    }
    if let Some(plan) = &a.plan {
        println!();
        let at = a.plan_at.map(style::ago).unwrap_or_default();
        println!(
            "{} {}",
            style::bold("Plan"),
            style::dim(&format!("(updated {at})"))
        );
        for line in plan.lines() {
            println!("  {}", style::plain(line));
        }
    }
}

fn print_note(n: &AutopilotNote) {
    let author = n.author.as_ref().map_or("Someone", |a| a.name.as_str());
    let seen = if n.seen_at.is_none() {
        style::blue(" · not seen yet")
    } else {
        String::new()
    };
    println!(
        "  {} {}{}",
        style::dim(&n.id),
        style::dim(&format!("{author}, {}", style::ago(n.created_at))),
        seen
    );
    for line in n.text.lines() {
        println!("    {}", style::plain(line));
    }
}

fn print_ask(ask: &AutopilotAsk) {
    let what = match &ask.secret {
        Some(s) => format!(
            "needs the secret {} for {}: {}",
            style::bold(&s.name),
            s.project_slug,
            ask.text
        ),
        None => ask.text.clone(),
    };
    let state = match ask.status {
        AutopilotAskStatus::Open => style::blue("open"),
        AutopilotAskStatus::Answered => style::dim("answered"),
        AutopilotAskStatus::Declined => style::dim("declined"),
        AutopilotAskStatus::Withdrawn => style::dim("taken back"),
        AutopilotAskStatus::Other => style::dim("other"),
    };
    println!(
        "  {} {} {}",
        style::dim(&ask.id),
        state,
        style::dim(&style::ago(ask.created_at))
    );
    println!("    {}", style::plain(&what));
    if !ask.options.is_empty() {
        println!(
            "    {}",
            style::dim(
                &ask.options
                    .iter()
                    .map(|o| format!("[{o}]"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        );
    }
    if let Some(answer) = &ask.answer {
        let by = ask
            .answered_by
            .as_ref()
            .map_or("Someone", |u| u.name.as_str());
        println!("    {}", style::dim(&format!("{by}: {answer}")));
    }
}

fn print_log(log: &AutopilotLog) {
    if log.entries.is_empty() {
        println!("Nothing yet.");
    }
    for e in &log.entries {
        // Who did it, unless the line names them already.
        let by =
            e.by.as_ref()
                .filter(|u| !e.text.starts_with(u.name.as_str()))
                .map(|u| format!(" · {}", u.name))
                .unwrap_or_default();
        // A session in another of its projects goes by that project's slug.
        let session = e
            .session
            .as_ref()
            .map(|s| {
                format!(
                    " · {}#{}",
                    e.project_slug.as_deref().unwrap_or_default(),
                    s.number
                )
            })
            .unwrap_or_default();
        println!(
            "{} {}",
            style::dim(&format!("{:>8}{by}{session}", style::ago(e.at))),
            style::plain(e.text.lines().next().unwrap_or_default())
        );
        for line in e.text.lines().skip(1) {
            println!("         {}", style::plain(line));
        }
    }
    if log.has_more {
        println!("{}", style::dim("… --limit lists more"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_brief_keeps_its_lines() {
        assert_eq!(
            block("# Autopilot\n\n- [ ] One\n- [x] Two\u{202E}"),
            "# Autopilot\n\n- [ ] One\n- [x] Two"
        );
    }

    #[test]
    fn where_a_fresh_session_takes_over_reads_as_people_type_it() {
        assert_eq!(handoff_arg("250k"), Ok(250_000));
        assert_eq!(handoff_arg("1M"), Ok(1_000_000));
        assert_eq!(handoff_arg("never"), Ok(0));
        assert_eq!(handoff_arg("Off"), Ok(0));
        assert!(handoff_arg("50k").is_err_and(|e| e.contains("100k to 1m")));
        assert!(handoff_arg("lots").is_err());
    }

    #[test]
    fn powers_read_as_people_type_them() {
        let project = AutopilotScope::Project;
        assert_eq!(
            parse_powers("settings, repositories", project).ok(),
            Some(vec![AutopilotPower::Settings, AutopilotPower::Repos])
        );
        assert_eq!(parse_powers("none", project).ok(), Some(Vec::new()));
        assert!(parse_powers("fly", project).is_err());
    }

    #[test]
    fn all_is_every_power_of_its_scope() {
        let all = |scope| parse_powers("all", scope).ok().unwrap_or_default();
        assert!(!all(AutopilotScope::Project).contains(&AutopilotPower::Projects));
        assert!(all(AutopilotScope::Project).contains(&AutopilotPower::Merge));
        assert_eq!(all(AutopilotScope::Project).len(), 5);
        assert_eq!(all(AutopilotScope::Workspace).len(), 6);
        // What the control plane takes for a project's autopilot.
        assert!(
            fctl_domain::autopilot::powers(AutopilotScope::Project, &all(AutopilotScope::Project))
                .is_ok()
        );
    }
}
