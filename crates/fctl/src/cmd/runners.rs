//! `fctl runners …`: the runners you see; approving and rejecting the ones
//! that wait for you (in workspaces you are an admin of, and your own);
//! managing runners as machines (yours, your workspaces' own as their
//! admin, every one as an instance admin); and workspaces' enrollment
//! tokens, with which machines enroll themselves as their runners, or as
//! yours.

use crate::cmd::admin::{drain_line, driver_line, find_runner, low_disk_line, still_running};
use crate::config;
use crate::style;
use anyhow::{Result, bail};
use clap::Subcommand;
use fctl_client::Api;
use fctl_domain::enrollment;
use fctl_proto::api::{
    CreateEnrollmentTokenRequest, EnrollmentToken, Placement, PoolDemand, Runner,
    UpdateEnrollmentTokenRequest, UpdateRunnerRequest, WorkspaceDemand,
};
use std::io::IsTerminal;

#[derive(Subcommand)]
pub enum RunnersCmd {
    /// The runners you see: yours, your workspaces', and, for instance
    /// admins, every one (the default).
    #[command(visible_alias = "ls")]
    List {
        /// Only this workspace's (its slug or id).
        #[arg(short, long)]
        workspace: Option<String>,
        /// Only the ones that wait for approval.
        #[arg(long)]
        pending: bool,
    },
    /// Approve a runner that waits for approval: one that enrolled in a
    /// workspace you are an admin of, which runs everyone's sessions there
    /// from now on, or one of yours that enrolled for your own sessions.
    /// Its host takes them at once.
    Approve {
        /// The runner's name or id.
        runner: String,
    },
    /// Reject a runner that waits for approval: it is removed, and its
    /// token stops working.
    Reject {
        /// The runner's name or id.
        runner: String,
    },
    /// Rename a runner.
    Rename {
        /// The runner's name or id.
        runner: String,
        /// Its new name.
        name: String,
    },
    /// Drain a runner: it takes no new sessions; the ones it has carry on,
    /// and a message resumes them there.
    Drain {
        /// The runner's name or id.
        runner: String,
    },
    /// Undrain a runner: it takes new sessions again.
    Undrain {
        /// The runner's name or id.
        runner: String,
    },
    /// Disable a runner: it disconnects, and its token works again only
    /// once it is enabled.
    Disable {
        /// The runner's name or id.
        runner: String,
    },
    /// Enable a disabled runner.
    Enable {
        /// The runner's name or id.
        runner: String,
    },
    /// Remove a runner: its token stops working, and the sessions it runs
    /// stop and resume on another runner. It asks first in a terminal.
    #[command(visible_alias = "rm")]
    Remove {
        /// The runner's name or id.
        runner: String,
        /// Don't ask.
        #[arg(long, short)]
        yes: bool,
    },
    /// Replace a runner's token (yours, or one of a workspace you are an
    /// admin of): the old one stops working at once. Prints the new one,
    /// which `fctl host install --token` takes on the machine.
    Token {
        /// The runner's name or id.
        runner: String,
    },
    /// A workspace's runner pools: the runners each enrollment token
    /// enrolled, with their labels and placement and what each needs now.
    /// Every one for its admins, your own for its members.
    #[command(after_help = "Examples:
  fctl runners pools -w acme
  fctl runners demand -w acme --pool \"AWS large\"   # one pool, in detail")]
    Pools {
        /// The workspace (its slug or id; default from `fctl workspaces
        /// use`, else your only one).
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// What a pool needs now, for whatever scales it: its runners, its
    /// slots, the sessions that wait that it could run, and how many
    /// runners would run them all (`wanted`). Signed in, a workspace's
    /// pools; with an enrollment token in FCTL_ENROLL_TOKEN, and no
    /// sign-in, that token's pool.
    #[command(after_help = "Examples:
  fctl runners demand -w acme
  fctl runners demand -w acme --pool \"AWS large\" --json   # what KEDA's metrics-api scaler reads
  fctl runners demand -w acme --watch                    # live, every 5 seconds
  FCTL_SERVER=https://app.fctl.sh FCTL_ENROLL_TOKEN=fctl_et_… fctl runners demand --cloudwatch

--cloudwatch puts WaitingSessions, BusySlots and WantedRunners into
CloudWatch with the AWS CLI (`aws cloudwatch put-metric-data`), under the
namespace fctl with the pool's name as the Pool dimension (its id where
the name isn't ASCII, which CloudWatch takes only): once, or every
--every with --watch.")]
    Demand {
        /// The workspace (its slug or id; default from `fctl workspaces
        /// use`, else your only one). Not with an enrollment token.
        #[arg(short, long)]
        workspace: Option<String>,
        /// Only this pool: its enrollment token's name or id.
        #[arg(long)]
        pool: Option<String>,
        /// Keep it live: read it again every `--every`, until Ctrl-C.
        #[arg(long)]
        watch: bool,
        /// How often `--watch` reads it: 5s (60s with --cloudwatch).
        #[arg(long, value_name = "DURATION", value_parser = seconds_arg)]
        every: Option<u64>,
        /// Put the numbers into CloudWatch (needs the AWS CLI and
        /// cloudwatch:PutMetricData).
        #[arg(long)]
        cloudwatch: bool,
        /// The CloudWatch namespace.
        #[arg(long, default_value = "fctl", requires = "cloudwatch")]
        namespace: String,
        /// Read the pool of this enrollment token rather than signing in,
        /// unless --workspace or --pool says which.
        #[arg(long, env = "FCTL_ENROLL_TOKEN", hide_env_values = true, hide = true)]
        enroll_token: Option<String>,
    },
    /// A workspace's enrollment tokens: machines enroll themselves with one
    /// (`fctl host install --enroll`), without anyone signing in on them,
    /// as autoscaling creates them, and the runners each enrolls are a
    /// pool, with its labels and placement. Its admins make them for
    /// everyone's sessions, and any member for their own (`--personal`).
    #[command(
        name = "enrollment-tokens",
        visible_alias = "enroll-tokens",
        after_help = "Examples:
  fctl runners enrollment-tokens -w acme
  fctl runners enrollment-tokens create \"Build fleet\" -w acme --expires 90d --max-runners 20 --remove-offline-after 5m
  fctl runners enrollment-tokens create \"AWS large\" -w acme --labels aws,large --max-runners 20 --remove-offline-after 5m
  fctl runners enrollment-tokens create \"Contractors\" -w acme --approval --expires 7d
  fctl runners enrollment-tokens create \"My cluster\" -w acme --personal --remove-offline-after 5m
  fctl runners enrollment-tokens set \"AWS large\" -w acme --placement spread --max-runners 40
  fctl runners enrollment-tokens revoke \"Build fleet\" -w acme

On each machine, without signing in:
  FCTL_SERVER=https://app.fctl.sh FCTL_ENROLL_TOKEN=fctl_et_… fctl host install"
    )]
    EnrollmentTokens {
        #[command(subcommand)]
        cmd: Option<EnrollmentTokensCmd>,
        /// The workspace (its slug or id; default from `fctl workspaces
        /// use`, else your only one).
        #[arg(short, long, global = true)]
        workspace: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum EnrollmentTokensCmd {
    /// The workspace's enrollment tokens, newest first (the default): every
    /// one for its admins, your own for its members.
    #[command(visible_alias = "ls")]
    List,
    /// Make an enrollment token. Prints its secret, once, on stdout.
    Create {
        /// What it is for, which the runners it enrolls show: "Build fleet".
        name: String,
        /// For your own sessions: the runners it enrolls are yours and run
        /// only your sessions in the workspace, so they count as your
        /// runners for Only on my runners and a Claude token you keep on
        /// your runners. A member's always is; an admin's runs everyone's
        /// sessions without it.
        #[arg(long)]
        personal: bool,
        /// How long it works: 1h, 7d, 90d, 1y, or never.
        #[arg(long, default_value = "90d", value_parser = hours_arg)]
        expires: Hours,
        /// The most runners it enrolls that may be there at once (rejected
        /// and removed ones don't count). Default: no limit.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=1000))]
        max_runners: Option<u32>,
        /// The runners it enrolls wait for approval before they run
        /// anything: by an admin of the workspace, or by you with
        /// `--personal`.
        #[arg(long)]
        approval: bool,
        /// Remove the runners it enrolls once they have been offline this
        /// long, as machines autoscaling destroys are: 1m, 5m, 15m, 1h, 1d,
        /// 7d (time the control plane is down doesn't count). Default: they
        /// stay until someone removes them.
        #[arg(long, value_name = "DURATION", value_parser = minutes_arg)]
        remove_offline_after: Option<Minutes>,
        /// Labels every runner it enrolls carries, beside its host.toml's:
        /// its runners are a pool, which projects choose by them (`fctl
        /// projects set --runners`). aws,large.
        #[arg(long, value_name = "LABELS", value_delimiter = ',')]
        labels: Vec<String>,
        /// How its runners take sessions: spread (the least busy first) or
        /// packed (the busiest with room, so that idle ones can go).
        /// Default: packed with --remove-offline-after, else spread.
        #[arg(long, value_name = "spread|packed", value_parser = placement_arg)]
        placement: Option<Placement>,
    },
    /// Change an enrollment token's pool: its name, labels, placement, or
    /// how many runners it keeps. Its runners carry the new labels and
    /// placement at once.
    Set {
        /// Its id (enr_…) or name.
        token: String,
        /// Its new name.
        #[arg(long)]
        name: Option<String>,
        /// Its runners' labels, replacing the ones it has; "" for none.
        #[arg(long, value_name = "LABELS", value_delimiter = ',')]
        labels: Option<Vec<String>>,
        /// spread or packed.
        #[arg(long, value_name = "spread|packed", value_parser = placement_arg)]
        placement: Option<Placement>,
        /// The most runners it keeps enrolled at once, or none for no
        /// limit.
        #[arg(long, value_name = "N|none", value_parser = max_runners_arg)]
        max_runners: Option<u32>,
    },
    /// Revoke an enrollment token: it enrolls no more runners. The ones it
    /// enrolled stay, and the ones that wait for approval still wait.
    #[command(visible_alias = "rm")]
    Revoke {
        /// Its id (enr_…) or name.
        token: String,
    },
}

/// A duration in hours, or none (`never`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hours(Option<u32>);

/// `90d` and the like; `never`.
fn hours_arg(s: &str) -> std::result::Result<Hours, String> {
    enrollment::parse_hours(s).map(Hours)
}

/// A duration in minutes, or none (`never`, `keep`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Minutes(Option<u32>);

/// `5m`, `1h` and the like; `keep`.
fn minutes_arg(s: &str) -> std::result::Result<Minutes, String> {
    enrollment::parse_minutes(s).map(Minutes)
}

/// `spread` or `packed`.
fn placement_arg(s: &str) -> std::result::Result<Placement, String> {
    Placement::parse(&s.to_lowercase()).ok_or_else(|| format!("{s:?}: use spread or packed"))
}

/// A number of runners, or `none` (0, no limit).
fn max_runners_arg(s: &str) -> std::result::Result<u32, String> {
    if matches!(s.trim().to_lowercase().as_str(), "none" | "any" | "0") {
        return Ok(0);
    }
    match s.trim().parse::<u32>() {
        Ok(n) if (1..=enrollment::MAX_RUNNERS).contains(&n) => Ok(n),
        _ => Err(format!(
            "{s:?}: use 1 to {} runners, or none",
            enrollment::MAX_RUNNERS
        )),
    }
}

/// `5s`, `1m`, `90s`: whole seconds, at least one.
fn seconds_arg(s: &str) -> std::result::Result<u64, String> {
    let t = s.trim().to_lowercase();
    let (n, per) = if let Some(n) = t.strip_suffix('s') {
        (n, 1)
    } else if let Some(n) = t.strip_suffix('m') {
        (n, 60)
    } else if let Some(n) = t.strip_suffix('h') {
        (n, 3600)
    } else {
        return Err(format!("{s:?} needs a unit: 5s, 1m or 1h"));
    };
    match n.trim().parse::<u64>() {
        Ok(n) if n > 0 => Ok(n * per),
        _ => Err(format!("{s:?} isn't a duration: use 5s, 1m or 1h")),
    }
}

pub async fn run(server: Option<&str>, cmd: RunnersCmd, json: bool) -> Result<()> {
    // A pool's demand reads with its enrollment token, where nobody signs
    // in: an autoscaler's exporter. Naming a workspace or a pool reads as
    // the person signed in, whatever the environment holds.
    if let RunnersCmd::Demand {
        enroll_token: Some(secret),
        workspace: None,
        pool: None,
        watch,
        every,
        cloudwatch,
        namespace,
    } = cmd
    {
        let api = Api::new(&config::server(server)?, Some(secret.trim().to_owned()));
        let show = Show {
            watch,
            every,
            cloudwatch,
            namespace,
            json,
            single: true,
        };
        return show
            .run(|| async {
                let p = api.pool_demand().await?;
                let at = p.at;
                Ok(WorkspaceDemand {
                    waiting: p.waiting.clone(),
                    unpooled: 0,
                    pools: vec![p],
                    at,
                })
            })
            .await;
    }
    let api = config::api(server)?;
    match cmd {
        RunnersCmd::Pools { workspace } => {
            let w = crate::cmd::workspaces::resolve(&api, workspace).await?;
            let demand = api.workspace_demand(w.id.as_str(), None).await?;
            if json {
                return style::print_json(&demand);
            }
            if demand.pools.is_empty() {
                println!(
                    "{}",
                    style::dim(&format!(
                        "{} has no pools yet: the runners an enrollment token enrolls are one. `fctl runners enrollment-tokens create NAME -w {} --labels …` makes one.",
                        w.name, w.slug
                    ))
                );
                return Ok(());
            }
            for p in &demand.pools {
                print_pool(p, false);
            }
            print_unpooled(&demand);
            Ok(())
        }
        RunnersCmd::Demand {
            workspace,
            pool,
            watch,
            every,
            cloudwatch,
            namespace,
            enroll_token: _,
        } => {
            let w = crate::cmd::workspaces::resolve(&api, workspace).await?;
            let show = Show {
                watch,
                every,
                cloudwatch,
                namespace,
                json,
                single: pool.is_some(),
            };
            let id = w.id.as_str();
            show.run(|| async { Ok(api.workspace_demand(id, pool.as_deref()).await?) })
                .await
        }
        RunnersCmd::List { workspace, pending } => {
            let mut runners = match &workspace {
                Some(w) => {
                    let w = crate::cmd::workspaces::resolve(&api, Some(w.clone())).await?;
                    api.workspace_runners(w.id.as_str()).await?.runners
                }
                None => api.runners().await?.runners,
            };
            if pending {
                runners.retain(|r| r.pending_until.is_some());
            }
            if json {
                return style::print_json(&fctl_proto::api::RunnerList { runners });
            }
            if runners.is_empty() {
                println!(
                    "{}",
                    style::dim(if pending {
                        "No runner waits for approval."
                    } else {
                        "No runners yet. `fctl host install` registers this machine; a workspace's admins make enrollment tokens for many (`fctl runners enrollment-tokens create`)."
                    })
                );
            }
            for r in &runners {
                print_runner(r);
            }
            if runners.iter().any(|r| r.pending_until.is_some()) {
                println!(
                    "{}",
                    style::dim(
                        "`fctl runners approve <name or id>` lets one run its workspace's sessions; `fctl runners reject` removes it."
                    )
                );
            }
            Ok(())
        }
        RunnersCmd::Approve { runner } => {
            let r = find_runner(&api, &runner).await?;
            if r.pending_until.is_none() && r.workspace.is_some() {
                if json {
                    return style::print_json(&r);
                }
                println!(
                    "{} {} was approved already: it runs {} in {}.",
                    style::accent("✓"),
                    style::bold(&r.name),
                    whose_sessions(&r),
                    workspace_name(&r)
                );
                return Ok(());
            }
            let r = api.approve_runner(r.id.as_str()).await?;
            if json {
                return style::print_json(&r);
            }
            println!(
                "{} Approved {}: it runs {} in {} from now on.",
                style::accent("✓"),
                style::bold(&r.name),
                whose_sessions(&r),
                workspace_name(&r)
            );
            Ok(())
        }
        RunnersCmd::Reject { runner } => {
            let r = find_runner(&api, &runner).await?;
            let ok = api.reject_runner(r.id.as_str()).await?;
            if json {
                return style::print_json(&ok);
            }
            println!(
                "{} Rejected {}: it is removed, and its token no longer works.",
                style::accent("✓"),
                style::bold(&r.name)
            );
            Ok(())
        }
        RunnersCmd::Rename { runner, name } => {
            let r = find_runner(&api, &runner).await?;
            let r = update(&api, &r, |u| u.name = Some(name.clone())).await?;
            if json {
                return style::print_json(&r);
            }
            println!("{} Renamed it {}", style::accent("✓"), style::bold(&r.name));
            Ok(())
        }
        RunnersCmd::Drain { runner } => {
            let r = find_runner(&api, &runner).await?;
            let r = update(&api, &r, |u| u.draining = Some(true)).await?;
            if json {
                return style::print_json(&r);
            }
            println!(
                "{} Draining {}: no new sessions start on it.",
                style::accent("✓"),
                style::bold(&r.name)
            );
            println!("{}", style::dim(&still_running(r.active_sessions, "there")));
            Ok(())
        }
        RunnersCmd::Undrain { runner } => {
            let r = find_runner(&api, &runner).await?;
            let r = update(&api, &r, |u| u.draining = Some(false)).await?;
            if json {
                return style::print_json(&r);
            }
            println!(
                "{} {} takes new sessions again.",
                style::accent("✓"),
                style::bold(&r.name)
            );
            Ok(())
        }
        RunnersCmd::Disable { runner } => {
            let r = find_runner(&api, &runner).await?;
            let r = update(&api, &r, |u| u.enabled = Some(false)).await?;
            if json {
                return style::print_json(&r);
            }
            println!(
                "{} Disabled {}: it is disconnected, and takes no sessions until `fctl runners enable {}`.",
                style::accent("✓"),
                style::bold(&r.name),
                r.id
            );
            Ok(())
        }
        RunnersCmd::Enable { runner } => {
            let r = find_runner(&api, &runner).await?;
            let r = update(&api, &r, |u| u.enabled = Some(true)).await?;
            if json {
                return style::print_json(&r);
            }
            println!(
                "{} Enabled {}: its host connects again with its token.",
                style::accent("✓"),
                style::bold(&r.name)
            );
            Ok(())
        }
        RunnersCmd::Remove { runner, yes } => {
            let r = find_runner(&api, &runner).await?;
            if !yes && !json && std::io::stdin().is_terminal() {
                let sessions = match r.active_sessions {
                    0 => String::new(),
                    n => format!(
                        " The {} it runs stop{}, and resume on another runner.",
                        style::count(n, "session", "sessions"),
                        if n == 1 { "s" } else { "" }
                    ),
                };
                println!(
                    "Remove {}? Its token stops working.{sessions}",
                    style::bold(&r.name)
                );
                if !confirm("Remove it? [y/N] ")? {
                    println!("Nothing changed.");
                    return Ok(());
                }
            }
            let ok = api.delete_runner(r.id.as_str()).await?;
            if json {
                return style::print_json(&ok);
            }
            println!("{} Removed {}", style::accent("✓"), style::bold(&r.name));
            Ok(())
        }
        RunnersCmd::Token { runner } => {
            let r = find_runner(&api, &runner).await?;
            let reg = api.replace_runner_token(r.id.as_str()).await?;
            if json {
                return style::print_json(&reg);
            }
            println!("{}", reg.token);
            eprintln!(
                "{}",
                style::dim(&format!(
                    "Shown once. {}'s old token no longer works: on the machine, `fctl host install --token <this>` puts the new one in place.",
                    reg.runner.name
                ))
            );
            Ok(())
        }
        RunnersCmd::EnrollmentTokens { cmd, workspace } => {
            let w = crate::cmd::workspaces::resolve(&api, workspace).await?;
            enrollment_tokens(&api, &w, cmd.unwrap_or(EnrollmentTokensCmd::List), json).await
        }
    }
}

async fn enrollment_tokens(
    api: &Api,
    w: &fctl_proto::api::Workspace,
    cmd: EnrollmentTokensCmd,
    json: bool,
) -> Result<()> {
    let id = w.id.as_str();
    match cmd {
        EnrollmentTokensCmd::List => {
            let list = api.enrollment_tokens(id).await?;
            if json {
                return style::print_json(&list);
            }
            let me = api.me().await.ok().map(|m| m.user.id);
            if list.tokens.is_empty() {
                println!(
                    "{}",
                    style::dim(&format!(
                        "{} has no enrollment tokens yet. `fctl runners enrollment-tokens create NAME -w {}` makes one.",
                        w.name, w.slug
                    ))
                );
            }
            for t in &list.tokens {
                print_token(t, me.as_ref());
            }
            Ok(())
        }
        EnrollmentTokensCmd::Create {
            name,
            personal,
            expires,
            max_runners,
            approval,
            remove_offline_after,
            labels,
            placement,
        } => {
            let created = api
                .create_enrollment_token(
                    id,
                    &CreateEnrollmentTokenRequest {
                        name,
                        // Without it, the control plane decides by role:
                        // a member's is for their own sessions.
                        personal: personal.then_some(true),
                        expires_in_hours: expires.0,
                        max_runners,
                        approval: Some(approval),
                        remove_offline_after_minutes: remove_offline_after.and_then(|m| m.0),
                        labels: (!labels.is_empty()).then_some(labels),
                        placement,
                    },
                )
                .await?;
            if json {
                return style::print_json(&created);
            }
            println!("{}", created.secret);
            let t = &created.token;
            let whose = if t.personal {
                enrollment::Whose::Yours
            } else {
                enrollment::Whose::Everyone
            };
            eprintln!(
                "{}",
                style::dim(&format!(
                    "{} {}",
                    enrollment::describe(
                        whose,
                        t.approval,
                        t.max_runners,
                        t.remove_offline_after_minutes
                    ),
                    expiry(t.expires_at)
                ))
            );
            eprintln!("{}", style::dim(&pool_line(t)));
            eprintln!(
                "{}",
                style::dim(&format!(
                    "Shown once. On each machine: FCTL_SERVER={} FCTL_ENROLL_TOKEN=<this> fctl host install",
                    api.base()
                ))
            );
            Ok(())
        }
        EnrollmentTokensCmd::Set {
            token,
            name,
            labels,
            placement,
            max_runners,
        } => {
            let list = api.enrollment_tokens(id).await?;
            let t = find_token(&list.tokens, &token, w, "change")?;
            if name.is_none() && labels.is_none() && placement.is_none() && max_runners.is_none() {
                bail!("Say what to change: --name, --labels, --placement or --max-runners.");
            }
            let labels = labels.map(|l| {
                l.into_iter()
                    .map(|l| l.trim().to_owned())
                    .filter(|l| !l.is_empty())
                    .collect()
            });
            let t = api
                .update_enrollment_token(
                    id,
                    t.id.as_str(),
                    &UpdateEnrollmentTokenRequest {
                        name,
                        labels,
                        placement,
                        max_runners,
                    },
                )
                .await?;
            if json {
                return style::print_json(&t);
            }
            println!(
                "{} Changed {}: {}",
                style::accent("✓"),
                style::bold(&t.name),
                match t.runners {
                    0 => "the runners it enrolls follow it.".to_owned(),
                    1 => "its runner follows it at once.".to_owned(),
                    n => format!("its {n} runners follow it at once."),
                }
            );
            println!("  {}", style::dim(&pool_line(&t)));
            Ok(())
        }
        EnrollmentTokensCmd::Revoke { token } => {
            let list = api.enrollment_tokens(id).await?;
            let t = find_token(&list.tokens, &token, w, "revoke")?;
            let t = api.revoke_enrollment_token(id, t.id.as_str()).await?;
            if json {
                return style::print_json(&t);
            }
            println!(
                "{} Revoked {}: it enrolls no more runners. The {} it enrolled stay.",
                style::accent("✓"),
                style::bold(&t.name),
                style::count(t.runners, "runner", "runners")
            );
            Ok(())
        }
    }
}

/// The enrollment token of `tokens` that `wanted` names, by its id or
/// name: of several with that name, the one that still enrolls, for
/// `verb` (`revoke`, `change`).
fn find_token<'a>(
    tokens: &'a [EnrollmentToken],
    wanted: &str,
    w: &fctl_proto::api::Workspace,
    verb: &str,
) -> Result<&'a EnrollmentToken> {
    let wanted = wanted.trim();
    let matching: Vec<&EnrollmentToken> = tokens
        .iter()
        .filter(|t| t.id.as_str() == wanted || t.name == wanted)
        .collect();
    let live: Vec<&&EnrollmentToken> = matching.iter().filter(|t| t.revoked_at.is_none()).collect();
    Ok(match (matching.as_slice(), live.as_slice()) {
        (_, [t]) => t,
        ([t], []) => t,
        ([], _) => bail!(
            "{} has no enrollment token {wanted}. `fctl runners enrollment-tokens -w {}` lists them.",
            w.name,
            w.slug
        ),
        _ => bail!(
            "{} enrollment tokens are called {wanted}: {verb} one by its id (`fctl runners enrollment-tokens -w {}` lists them)",
            live.len(),
            w.slug
        ),
    })
}

/// A pool's labels and placement, in a line: `Labels aws, large. Packed:
/// fills one runner before the next, so that idle ones can go.`
fn pool_line(t: &EnrollmentToken) -> String {
    let labels = if t.labels.is_empty() {
        "No labels: projects that ask for none run on its runners".to_owned()
    } else {
        format!(
            "Its runners carry {}",
            fctl_domain::pools::labels_words(&t.labels)
        )
    };
    format!(
        "{labels}. {}.",
        fctl_domain::pools::placement_words(t.placement)
    )
}

/// How `fctl runners demand` shows what it reads: once or live, as text,
/// JSON or CloudWatch metrics.
struct Show {
    watch: bool,
    every: Option<u64>,
    cloudwatch: bool,
    namespace: String,
    json: bool,
    /// One pool was asked for (`--pool`, or an enrollment token's): its
    /// JSON is that pool's, as the token's endpoint answers, rather than
    /// the workspace's list.
    single: bool,
}

impl Show {
    /// Reads demand with `read`, and shows it: once, or every `every`
    /// while it watches.
    async fn run<F, Fut>(self, read: F) -> Result<()>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<WorkspaceDemand>>,
    {
        let every = std::time::Duration::from_secs(self.every.unwrap_or(if self.cloudwatch {
            60
        } else {
            5
        }));
        loop {
            let demand = read().await;
            match (&demand, self.watch) {
                (Ok(d), false) => self.show(d)?,
                // Watching, a read or a put that fails says so and tries
                // again: an exporter runs for good, through a throttled
                // call or credentials that expire.
                (Ok(d), true) => {
                    if let Err(e) = self.show(d) {
                        eprintln!("{}", style::red(&format!("{e:#}")));
                    }
                }
                (Err(e), true) => eprintln!("{}", style::red(&format!("{e:#}"))),
                (Err(_), false) => return demand.map(|_| ()),
            }
            if !self.watch {
                return Ok(());
            }
            tokio::select! {
                () = tokio::time::sleep(every) => {}
                _ = tokio::signal::ctrl_c() => return Ok(()),
            }
        }
    }

    fn show(&self, d: &WorkspaceDemand) -> Result<()> {
        if self.cloudwatch {
            for p in &d.pools {
                put_metrics(&self.namespace, p)?;
            }
            if !self.json {
                let pools: Vec<String> = d
                    .pools
                    .iter()
                    .map(|p| {
                        format!(
                            "{}: {} waiting, {} busy, {} wanted",
                            p.pool.name, p.waiting.sessions, p.slots.busy, p.wanted
                        )
                    })
                    .collect();
                println!(
                    "{} {} {}",
                    style::dim(&clock(d.at)),
                    style::accent("✓"),
                    pools.join("; ")
                );
            }
            return Ok(());
        }
        if self.json {
            // One line each, so that a watch reads as JSON lines; the pool
            // asked for as itself, as the enrollment token's endpoint
            // answers it.
            let line = match (self.single, d.pools.first()) {
                (true, Some(one)) => serde_json::to_string(one)?,
                _ => serde_json::to_string(d)?,
            };
            println!("{line}");
            return Ok(());
        }
        if self.watch && std::io::stdout().is_terminal() {
            // Clear the screen and start at its top.
            print!("\x1b[2J\x1b[H");
        }
        if d.pools.is_empty() {
            println!(
                "{}",
                style::dim("No pools: the runners an enrollment token enrolls are one.")
            );
        }
        for p in &d.pools {
            print_pool(p, true);
        }
        print_unpooled(d);
        if self.watch {
            println!(
                "{}",
                style::dim(&format!("As of {}. Ctrl-C stops.", clock(d.at)))
            );
        }
        Ok(())
    }
}

/// Puts a pool's WaitingSessions, BusySlots and WantedRunners into
/// CloudWatch, with its name as the Pool dimension ([`cloudwatch_pool`]),
/// with the AWS CLI.
fn put_metrics(namespace: &str, p: &PoolDemand) -> Result<()> {
    let data = cloudwatch_data(p);
    let out = std::process::Command::new("aws")
        .args(["cloudwatch", "put-metric-data", "--namespace", namespace])
        .arg("--metric-data")
        .arg(data.to_string())
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => anyhow::anyhow!(
                "--cloudwatch needs the AWS CLI (`aws`) on the PATH, with credentials that may cloudwatch:PutMetricData"
            ),
            _ => anyhow::anyhow!("running the AWS CLI: {e}"),
        })?;
    if !out.status.success() {
        bail!(
            "aws cloudwatch put-metric-data failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// A pool's Pool dimension in CloudWatch: its name, or its id where
/// CloudWatch wouldn't take the name, which it takes only as printable
/// ASCII, not all spaces, of at most 1,024 characters: a pool named "Büro"
/// is `enr_…` there.
fn cloudwatch_pool<'a>(id: &'a str, name: &'a str) -> &'a str {
    let takes = !name.trim().is_empty()
        && name.len() <= 1024
        && name.chars().all(|c| c.is_ascii() && !c.is_ascii_control());
    if takes { name } else { id }
}

/// The metric data `aws cloudwatch put-metric-data` takes for a pool.
fn cloudwatch_data(p: &PoolDemand) -> serde_json::Value {
    let pool = cloudwatch_pool(p.pool.id.as_str(), &p.pool.name);
    let dims = serde_json::json!([{ "Name": "Pool", "Value": pool }]);
    let metric = |name: &str, value: u32| {
        serde_json::json!({
            "MetricName": name,
            "Dimensions": dims,
            "Value": value,
            "Unit": "Count",
            "Timestamp": p.at / 1000,
        })
    };
    serde_json::json!([
        metric("WaitingSessions", p.waiting.sessions),
        metric("BusySlots", p.slots.busy),
        metric("WantedRunners", p.wanted),
    ])
}

/// One pool and what it needs now; `full`, with every count.
fn print_pool(p: &PoolDemand, full: bool) {
    let r = &p.runners;
    let limit = match p.pool.max_runners {
        Some(max) => format!(" of {max}"),
        None => String::new(),
    };
    let enrolled = r.online + r.draining + r.pending + r.starting + r.offline;
    let closed = if p.pool.closed {
        " · its token no longer enrolls"
    } else {
        ""
    };
    let whose = p
        .pool
        .owner
        .as_ref()
        .map(|u| format!(" · {}'s", u.name))
        .unwrap_or_default();
    let runners = match p.pool.max_runners {
        Some(_) => format!("{enrolled}{limit} runners"),
        None => style::count(enrolled, "runner", "runners"),
    };
    println!(
        "{}  {}",
        style::bold(&p.pool.name),
        style::dim(&format!(
            "{} · {runners}{whose}{closed} · {}",
            if p.pool.labels.is_empty() {
                "no labels".to_owned()
            } else {
                p.pool.labels.join(", ")
            },
            p.pool.id
        ))
    );
    println!(
        "  {}",
        fctl_domain::pools::placement_words(p.pool.placement)
    );
    let mut state = vec![format!("{} online ({} busy)", r.online, r.busy)];
    for (n, what) in [
        (r.starting, "starting"),
        (r.draining, "draining"),
        (r.pending, "waiting for approval"),
        (r.offline, "offline"),
    ] {
        if n > 0 || full {
            state.push(format!("{n} {what}"));
        }
    }
    println!("  runners  {}", state.join(" · "));
    println!(
        "  slots    {} of {} busy · {} free",
        p.slots.busy, p.slots.capacity, p.slots.free
    );
    let waiting = match (p.waiting.sessions, p.waiting.since) {
        (0, _) | (_, None) => "no session waits for it".to_owned(),
        (n, Some(since)) => format!(
            "{}, the longest for {}{}",
            style::count(n, "session", "sessions"),
            waited(p.at - since),
            reasons(&p.waiting.reasons)
        ),
    };
    println!("  waiting  {waiting}");
    println!(
        "  wants    {}{}",
        style::count(p.wanted, "runner", "runners"),
        match &p.shape {
            Some(s) if full => format!(
                ", {} sessions each ({} CPUs, {} as its runners last said)",
                s.capacity,
                s.cpus,
                fctl_domain::size::memory(s.memory_mb)
            ),
            None if p.wanted > 0 =>
                ": none of its runners has connected yet to say what it gives".to_owned(),
            _ => String::new(),
        }
    );
}

/// `14:05:09 UTC`.
fn clock(ms: i64) -> String {
    let s = (ms / 1000).rem_euclid(86_400);
    format!("{:02}:{:02}:{:02} UTC", s / 3600, s / 60 % 60, s % 60)
}

/// How long something has waited: `40s`, `2m`, `1h 5m`, `2d 3h`.
fn waited(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h {}m", s / 3600, s / 60 % 60),
        _ => format!("{}d {}h", s / 86_400, s / 3600 % 24),
    }
}

/// ` (busy 3, too_big 1)`.
fn reasons(reasons: &std::collections::BTreeMap<String, u32>) -> String {
    if reasons.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = reasons.iter().map(|(k, n)| format!("{k} {n}")).collect();
    format!(" ({})", parts.join(", "))
}

/// The sessions that wait which no pool could run, when there are some.
fn print_unpooled(d: &WorkspaceDemand) {
    if d.unpooled > 0 {
        println!(
            "{}",
            style::amber(&format!(
                "{} that no pool could run.",
                match d.unpooled {
                    1 => "1 session waits".to_owned(),
                    n => format!("{n} sessions wait"),
                }
            ))
        );
    }
}

/// Changes a runner with one PATCH.
async fn update(
    api: &Api,
    r: &Runner,
    change: impl FnOnce(&mut UpdateRunnerRequest),
) -> Result<Runner> {
    let mut req = UpdateRunnerRequest::default();
    change(&mut req);
    Ok(api.update_runner(r.id.as_str(), &req).await?)
}

fn workspace_name(r: &Runner) -> String {
    r.workspace
        .as_ref()
        .map_or_else(|| "its workspace".to_owned(), |w| w.name.clone())
}

/// Whose sessions a runner that enrolled runs: `everyone's sessions`, or,
/// for one a member enrolled for their own, `your sessions` or `Ada's
/// sessions`.
fn whose_sessions(r: &Runner) -> String {
    match &r.created_by {
        None => enrollment::Whose::Everyone,
        Some(_) if r.yours => enrollment::Whose::Yours,
        Some(u) => enrollment::Whose::Theirs(&u.name),
    }
    .sessions()
}

/// `expires in 89d 23h.`, `expired 2d ago.`, `never expires.`
fn expiry(expires_at: Option<i64>) -> String {
    match expires_at {
        None => "It never expires.".to_owned(),
        Some(at) if at <= style::now_ms() => format!("It expired {}.", style::ago(at)),
        Some(at) => format!("It expires {}.", style::until(at)),
    }
}

/// One runner, with whose it is and what it does now.
fn print_runner(r: &Runner) {
    let whose = match (&r.workspace, &r.created_by) {
        (Some(w), None) => format!("{}'s", w.name),
        (_, Some(_)) if r.yours => "yours".to_owned(),
        (_, Some(u)) => format!("{}'s", u.name),
        (None, None) => "nobody's".to_owned(),
    };
    println!(
        "{:<24} {:<8} {:<8} {}/{} sessions  {}",
        style::bold(&r.name),
        style::runner_status(r),
        r.driver.as_deref().unwrap_or_default(),
        r.active_sessions,
        r.capacity,
        style::dim(&format!("{whose} · {}", r.id))
    );
    if let Some(until) = r.pending_until {
        let from = r
            .enrollment
            .as_ref()
            .map(|e| {
                let token = e
                    .token_name
                    .as_deref()
                    .map(|t| format!(" with “{t}”"))
                    .unwrap_or_default();
                let address = match (&e.address, &e.location) {
                    (Some(a), Some(l)) => format!(" from {a} ({l})"),
                    (Some(a), None) => format!(" from {a}"),
                    _ => String::new(),
                };
                format!(" since it enrolled{token}{address}")
            })
            .unwrap_or_default();
        println!(
            "  {}",
            style::amber(&format!(
                "waits for approval{from}; removed {} unless approved",
                style::until(until)
            ))
        );
    } else if let Some(t) = r
        .enrollment
        .as_ref()
        .and_then(|e| e.token_name.as_deref())
        .or(r.pool.as_ref().map(|p| p.name.as_str()))
    {
        let place = match (&r.workspace, &r.created_by) {
            (Some(w), Some(_)) => format!(" in {}, for {}", w.name, whose_sessions(r)),
            _ => String::new(),
        };
        println!("  {}", style::dim(&format!("enrolled with “{t}”{place}")));
    }
    let facts: Vec<String> = [
        r.hostname.clone(),
        r.os.as_deref()
            .zip(r.arch.as_deref())
            .map(|(os, arch)| format!("{os}/{arch}")),
        r.version.as_ref().map(|v| format!("fctl {v}")),
        Some(match r.last_seen_at {
            Some(t) => format!("seen {}", style::ago(t)),
            None => "never connected".to_owned(),
        }),
    ]
    .into_iter()
    .flatten()
    .collect();
    println!("  {}", style::dim(&facts.join(" · ")));
    if !r.labels.is_empty() {
        println!(
            "  {}",
            style::dim(&format!("labels {}", r.labels.join(", ")))
        );
    }
    if let Some(d) = drain_line(r) {
        println!("  {}", style::dim(&d));
    }
    if let Some(d) = low_disk_line(r) {
        println!("  {d}");
    }
    if let Some(d) = driver_line(r) {
        println!("  {}", style::amber(&d));
    }
    if let Some(h) = r.remove_offline_after_minutes {
        println!(
            "  {}",
            style::dim(&format!(
                "removed once offline for {}",
                enrollment::minutes(h)
            ))
        );
    }
}

/// One enrollment token, with what it does and how much it was used, as
/// the person signed in (`me`) reads it.
fn print_token(t: &EnrollmentToken, me: Option<&fctl_proto::UserId>) {
    let now = style::now_ms();
    let state = match (t.revoked_at, t.expires_at) {
        (Some(_), _) => style::red("revoked"),
        (None, Some(at)) if at <= now => style::dim("expired"),
        _ => style::accent("active"),
    };
    let runners = match t.max_runners {
        Some(max) => format!("{} of {max} runners", t.runners),
        None => style::count(t.runners, "runner", "runners"),
    };
    let waiting = match t.pending {
        0 => String::new(),
        n => format!(", {n} waiting for approval"),
    };
    println!(
        "{:<24} {:<8} {runners}{waiting}  {}",
        style::bold(&t.name),
        state,
        style::dim(t.id.as_str())
    );
    let made = match &t.created_by {
        Some(u) => format!("Made by {} {}", u.name, style::ago(t.created_at)),
        None => format!("Made {}", style::ago(t.created_at)),
    };
    let used = t
        .last_used_at
        .map(|at| format!("; last enrolled a runner {}", style::ago(at)))
        .unwrap_or_default();
    let whose = match (&t.created_by, t.personal) {
        (_, false) => enrollment::Whose::Everyone,
        (Some(u), true) if Some(&u.id) == me => enrollment::Whose::Yours,
        (Some(u), true) => enrollment::Whose::Theirs(&u.name),
        (None, true) => enrollment::Whose::Theirs("its maker"),
    };
    println!(
        "  {}",
        style::dim(&format!(
            "{} {} {made}{used}.",
            enrollment::describe(
                whose,
                t.approval,
                t.max_runners,
                t.remove_offline_after_minutes
            ),
            expiry(t.expires_at)
        ))
    );
    println!("  {}", style::dim(&pool_line(t)));
}

/// Asks a yes-or-no question on the terminal.
fn confirm(question: &str) -> Result<bool> {
    use std::io::{BufRead, Write};
    print!("{question}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "Yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloudwatch_takes_a_pools_name_only_in_ascii() {
        use super::cloudwatch_pool;
        assert_eq!(cloudwatch_pool("enr_1", "AWS large"), "AWS large");
        for name in ["Büro", "東京", "", "   ", "tab\there"] {
            assert_eq!(cloudwatch_pool("enr_1", name), "enr_1", "{name:?}");
        }
        assert_eq!(cloudwatch_pool("enr_1", &"a".repeat(1025)), "enr_1");
    }

    #[test]
    fn durations_parse_for_enrollment_tokens() {
        assert_eq!(hours_arg("90d"), Ok(Hours(Some(2160))));
        assert_eq!(hours_arg("never"), Ok(Hours(None)));
        assert!(hours_arg("soon").is_err());
        assert_eq!(minutes_arg("5m"), Ok(Minutes(Some(5))));
        assert_eq!(minutes_arg("1h"), Ok(Minutes(Some(60))));
        assert_eq!(minutes_arg("keep"), Ok(Minutes(None)));
        assert!(minutes_arg("5").is_err());
    }

    #[test]
    fn expiry_says_when() {
        assert_eq!(expiry(None), "It never expires.");
        assert!(expiry(Some(style::now_ms() - 3 * 86_400_000)).starts_with("It expired 3d ago"));
        assert!(
            expiry(Some(style::now_ms() + 3 * 86_400_000 + 60_000)).starts_with("It expires in 3d")
        );
    }
}
