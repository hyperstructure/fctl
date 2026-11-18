//! `fctl webhooks`: addresses fctl posts a workspace's or a project's events
//! to as they happen (docs/automation.md#webhooks): adding and changing
//! them, their secrets, tests, what went out to each and resending it, the
//! events there are, and checking a post's signature as a receiver does.

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use fctl_client::Api;
use fctl_domain::hooks::{self as rules, Kind};
use fctl_proto::ProjectId;
use fctl_proto::hooks::{
    CreateWebhookRequest, ResendFailedRequest, TestWebhookRequest, UpdateWebhookRequest, Webhook,
    WebhookAttempt, WebhookDelivery, WebhookDeliveryState, WebhookOrigin,
};
use std::io::Read;
use std::path::PathBuf;

#[derive(Args)]
pub struct WebhooksArgs {
    /// A project's webhooks (slug or id) rather than the workspace's.
    #[arg(short, long, global = true, conflicts_with = "workspace")]
    pub project: Option<String>,
    /// The workspace (slug or id); default from `fctl workspaces use`, else
    /// your only one.
    #[arg(short, long, global = true)]
    pub workspace: Option<String>,
    #[command(subcommand)]
    pub cmd: Option<WebhooksCmd>,
}

/// What a webhook gets.
#[derive(Args, Default)]
pub struct WantsArgs {
    /// The events it gets, comma-separated: their names
    /// (session.finished), a group's (session.*, pull_request.*, review.*,
    /// schedule.*), or `all` (the default for a new one). `fctl webhooks
    /// events` lists them.
    #[arg(long, value_delimiter = ',', value_name = "EVENTS")]
    pub events: Vec<String>,
    /// A workspace's webhook: only these projects' events (slugs or ids,
    /// comma-separated); `all` for every project's, also those made later.
    #[arg(long, value_delimiter = ',', value_name = "PROJECTS")]
    pub projects: Vec<String>,
    /// Sessions' events, and their pull requests': only of sessions that
    /// people, schedules, an autopilot or tickets started (comma-separated:
    /// people, schedules, autopilot, tickets); `all` for any.
    #[arg(long, value_delimiter = ',', value_name = "WHO")]
    pub started_by: Vec<String>,
}

#[derive(Subcommand)]
pub enum WebhooksCmd {
    /// The workspace's webhooks and its projects' (its admins), or a
    /// project's (-p) with the workspace's that send its events (the
    /// default).
    #[command(visible_alias = "ls")]
    List,
    /// Add a webhook: the workspace's (its admins), or a project's (-p, its
    /// owners and maintainers). fctl first posts the address a challenge
    /// (type url_verification), which it answers with the challenge. Prints
    /// its signing secret, once.
    #[command(after_help = "Examples:
  fctl webhooks add https://hooks.example.com/fctl --name \"Deploy bot\" --events session.ready_for_review
  fctl webhooks add https://example.com/hook -p web --events session.question,session.ready_for_review,session.failed
  fctl webhooks add https://example.com/reviews --events review.* --projects web,api
  fctl webhooks add https://example.com/nightly --events session.started,session.stopped --started-by schedules")]
    Add {
        /// Where fctl posts its events: https://, a public host.
        url: String,
        /// What to call it; default: its host.
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        wants: WantsArgs,
        /// Add it off: it gets nothing until it's turned on.
        #[arg(long)]
        off: bool,
    },
    /// A webhook: what it gets, how its deliveries went lately and its
    /// newest ones.
    Show {
        /// Its id (whk_…), name or host.
        webhook: String,
    },
    /// Change a webhook, or turn it on or off.
    #[command(after_help = "Examples:
  fctl webhooks edit \"Deploy bot\" --events session.ready_for_review,review.finished
  fctl webhooks edit whk_01KC… --url https://hooks.example.com/v2
  fctl webhooks edit \"Deploy bot\" --on      # again, after fctl turned it off")]
    Edit {
        /// Its id (whk_…), name or host.
        webhook: String,
        /// Where fctl posts its events from now on, once the address answers
        /// fctl's challenge.
        #[arg(long)]
        url: Option<String>,
        /// A new name; empty for none.
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        wants: WantsArgs,
        /// Turn it on.
        #[arg(long, conflicts_with = "off")]
        on: bool,
        /// Turn it off: it gets nothing until it's turned on.
        #[arg(long)]
        off: bool,
    },
    /// A webhook's signing secret, on stdout; --rotate makes a new one, and
    /// the old one signs too for a day.
    Secret {
        /// Its id (whk_…), name or host.
        webhook: String,
        #[arg(long)]
        rotate: bool,
    },
    /// Post a sample of an event to a webhook now, marked as a test, and
    /// see how it went.
    Test {
        /// Its id (whk_…), name or host.
        webhook: String,
        /// The event to send a sample of.
        #[arg(long, default_value = "session.finished")]
        event: String,
    },
    /// A webhook's deliveries, newest first, from the last 30 days.
    Deliveries {
        /// Its id (whk_…), name or host.
        webhook: String,
        /// Only those that failed every attempt.
        #[arg(long, conflicts_with_all = ["pending", "delivered"])]
        failed: bool,
        /// Only those waiting for an attempt.
        #[arg(long, conflicts_with = "delivered")]
        pending: bool,
        /// Only those it took.
        #[arg(long)]
        delivered: bool,
        /// Only of this event.
        #[arg(long)]
        event: Option<String>,
        /// How many.
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// One delivery: the request as fctl posted it, every attempt and the
    /// start of the last answer.
    Delivery {
        /// Its id (whk_…), name or host.
        webhook: String,
        /// The delivery (whd_…).
        delivery: String,
    },
    /// Post a delivery again now (five times at most), or with --failed,
    /// all that failed since a time (the last day by default) in their
    /// turn.
    #[command(after_help = "Examples:
  fctl webhooks resend \"Deploy bot\" whd_01KC…
  fctl webhooks resend \"Deploy bot\" --failed --since 6h")]
    Resend {
        /// Its id (whk_…), name or host.
        webhook: String,
        /// The delivery (whd_…).
        #[arg(required_unless_present = "failed", conflicts_with = "failed")]
        delivery: Option<String>,
        /// All that failed since --since.
        #[arg(long)]
        failed: bool,
        /// With --failed: how far back, like 6h, 2d (at most 30d).
        #[arg(long, requires = "failed")]
        since: Option<String>,
    },
    /// Delete a webhook, and what went out to it.
    #[command(visible_alias = "delete")]
    Rm {
        /// Its id (whk_…), name or host.
        webhook: String,
    },
    /// The events webhooks get.
    Events,
    /// Check a post's signature as a receiver does: the body from a file
    /// (or `-`, stdin), with its webhook-id, webhook-timestamp and
    /// webhook-signature headers.
    #[command(after_help = "Example:
  fctl webhooks verify --secret whsec_… --id evt_01KC… --timestamp 1763400000 \\
    --signature \"v1,K5o…\" body.json")]
    Verify {
        /// The webhook's secret (whsec_…), or FCTL_WEBHOOK_SECRET.
        #[arg(long, env = "FCTL_WEBHOOK_SECRET", hide_env_values = true)]
        secret: String,
        /// The webhook-id header.
        #[arg(long)]
        id: String,
        /// The webhook-timestamp header.
        #[arg(long)]
        timestamp: String,
        /// The webhook-signature header.
        #[arg(long)]
        signature: String,
        /// Check the signature of a request saved earlier, whatever its age.
        /// Without it, one more than 5 minutes from this machine's clock is
        /// refused, as a receiver refuses a possible replay.
        #[arg(long)]
        any_age: bool,
        /// The body, as it came.
        body: PathBuf,
    },
}

pub async fn run(server: Option<&str>, args: WebhooksArgs, json: bool) -> Result<()> {
    if let Some(WebhooksCmd::Verify {
        secret,
        id,
        timestamp,
        signature,
        any_age,
        body,
    }) = &args.cmd
    {
        let mut bytes = Vec::new();
        if body.as_os_str() == "-" {
            std::io::stdin().read_to_end(&mut bytes)?;
        } else {
            bytes = std::fs::read(body).with_context(|| format!("reading {}", body.display()))?;
        }
        let now = style::now_ms() / 1000;
        return match verdict(secret, id, timestamp, &bytes, signature, now, *any_age) {
            Ok(said) => {
                println!("{said}");
                Ok(())
            }
            Err(said) => bail!(said),
        };
    }
    let api = config::api(server)?;
    let id = args
        .cmd
        .as_ref()
        .and_then(WebhooksCmd::webhook)
        .filter(|w| fctl_proto::WebhookId::is_valid(w));
    let place = match id {
        // Its id says where it is, unless a workspace or project is given.
        Some(id) if args.workspace.is_none() && args.project.is_none() => {
            Place::of_webhook(&api, id).await?
        }
        _ => Place::of(&api, &args).await?,
    };
    match args.cmd.unwrap_or(WebhooksCmd::List) {
        WebhooksCmd::List => {
            let list = place.list(&api).await?;
            if json {
                return style::print_json(&list);
            }
            if list.webhooks.is_empty() {
                println!(
                    "{} has no webhooks. Add one: fctl webhooks add https://… --events session.ready_for_review",
                    place.name
                );
                return Ok(());
            }
            let names = project_names(&api, &place, &list.webhooks).await;
            for (i, w) in list.webhooks.iter().enumerate() {
                if i > 0 {
                    println!();
                }
                print_webhook(w, &names);
            }
            println!(
                "\n{}",
                style::dim(&format!(
                    "{} of {} webhooks in the workspace",
                    list.count, list.limit
                ))
            );
            Ok(())
        }
        WebhooksCmd::Add {
            url,
            name,
            wants,
            off,
        } => {
            let events = events(&wants.events)?.unwrap_or_default();
            let projects = match &place.project {
                Some(_) if !wants.projects.is_empty() => {
                    bail!(
                        "A project's webhook gets its own project's events: leave --projects out."
                    )
                }
                Some(_) => Vec::new(),
                None => projects(&api, &place, &wants.projects)
                    .await?
                    .unwrap_or_default(),
            };
            let req = CreateWebhookRequest {
                url,
                name,
                events,
                projects,
                started_by: started_by(&wants.started_by)?.unwrap_or_default(),
                enabled: off.then_some(false),
            };
            let created = match &place.project {
                Some(p) => api.create_project_webhook(p, &req).await?,
                None => api.create_workspace_webhook(&place.workspace, &req).await?,
            };
            if json {
                return style::print_json(&created);
            }
            let w = &created.webhook;
            let names = project_names(&api, &place, std::slice::from_ref(w)).await;
            eprintln!(
                "Added {} ({}), whose address answered fctl's challenge: {}",
                style::bold(&title(w)),
                w.id,
                describe(w, &names)
            );
            eprintln!(
                "Its signing secret, shown once here ({} shows it again):",
                style::dim(&format!("fctl webhooks secret {}", w.id))
            );
            println!("{}", created.secret);
            eprintln!(
                "{}",
                style::dim(&format!(
                    "Send it a test: fctl webhooks test {} --event session.ready_for_review",
                    w.id
                ))
            );
            Ok(())
        }
        WebhooksCmd::Show { webhook } => {
            let id = place.find(&api, &webhook).await?;
            let w = api.get_webhook(&id).await?;
            let recent = api.webhook_deliveries(&id, None, None, None, 5).await?;
            if json {
                return style::print_json(
                    &serde_json::json!({ "webhook": w, "deliveries": recent.deliveries }),
                );
            }
            let names = project_names(&api, &place, std::slice::from_ref(&w)).await;
            print_webhook(&w, &names);
            if let Some(until) = w.previous_secret_until {
                println!(
                    "  {}",
                    style::dim(&format!(
                        "Its previous secret stops signing {}",
                        style::until(until)
                    ))
                );
            }
            if !recent.deliveries.is_empty() {
                println!("\n{}", style::bold("Newest deliveries"));
                for d in &recent.deliveries {
                    print_delivery_line(d);
                }
            }
            Ok(())
        }
        WebhooksCmd::Edit {
            webhook,
            url,
            name,
            wants,
            on,
            off,
        } => {
            let id = place.find(&api, &webhook).await?;
            let req = UpdateWebhookRequest {
                url,
                name,
                events: events(&wants.events)?,
                projects: projects(&api, &place, &wants.projects).await?,
                started_by: started_by(&wants.started_by)?,
                enabled: if on {
                    Some(true)
                } else if off {
                    Some(false)
                } else {
                    None
                },
            };
            let w = api.update_webhook(&id, &req).await?;
            if json {
                return style::print_json(&w);
            }
            let names = project_names(&api, &place, std::slice::from_ref(&w)).await;
            println!(
                "Saved {}: {}",
                style::bold(&title(&w)),
                describe(&w, &names)
            );
            if !w.enabled {
                println!(
                    "{}",
                    style::dim("It's off: it gets nothing until it's turned on (--on).")
                );
            }
            Ok(())
        }
        WebhooksCmd::Secret { webhook, rotate } => {
            let id = place.find(&api, &webhook).await?;
            let s = if rotate {
                api.rotate_webhook_secret(&id).await?
            } else {
                api.webhook_secret(&id).await?
            };
            if json {
                return style::print_json(&s);
            }
            println!("{}", s.secret);
            if let Some(until) = s.previous_until {
                eprintln!(
                    "{}",
                    style::dim(&format!(
                        "The one before it stops signing {}: switch the receiver to this one before then.",
                        style::until(until)
                    ))
                );
            }
            Ok(())
        }
        WebhooksCmd::Test { webhook, event } => {
            let id = place.find(&api, &webhook).await?;
            let event = event.trim().to_owned();
            if Kind::parse(&event).is_none() {
                bail!("fctl has no event called {event}: fctl webhooks events lists them.");
            }
            let d = api
                .test_webhook(
                    &id,
                    &TestWebhookRequest {
                        event: event.clone(),
                    },
                )
                .await?;
            if json {
                style::print_json(&d)?;
                if d.state != WebhookDeliveryState::Delivered {
                    std::process::exit(1);
                }
                return Ok(());
            }
            let to = d
                .request
                .as_ref()
                .map(|r| rules::host_of(&r.url))
                .unwrap_or_default();
            println!("Sent a test of {event} to {to}: {}", outcome(&d));
            if let Some(body) = d
                .last
                .as_ref()
                .and_then(|a| a.response.as_ref())
                .and_then(|r| r.body.as_deref())
            {
                println!(
                    "{}",
                    style::dim(&style::terminal_safe(&first_lines(body, 6)))
                );
            }
            if d.state != WebhookDeliveryState::Delivered {
                std::process::exit(1);
            }
            Ok(())
        }
        WebhooksCmd::Deliveries {
            webhook,
            failed,
            pending,
            delivered,
            event,
            limit,
        } => {
            let id = place.find(&api, &webhook).await?;
            let state = if failed {
                Some("failed")
            } else if pending {
                Some("pending")
            } else if delivered {
                Some("delivered")
            } else {
                None
            };
            if let Some(e) = &event
                && Kind::parse(e).is_none()
            {
                bail!("fctl has no event called {e}: fctl webhooks events lists them.");
            }
            let list = api
                .webhook_deliveries(&id, state, event.as_deref(), None, limit)
                .await?;
            if json {
                return style::print_json(&list);
            }
            if list.deliveries.is_empty() {
                println!("No deliveries like that in the last 30 days.");
                return Ok(());
            }
            for d in &list.deliveries {
                print_delivery_line(d);
            }
            Ok(())
        }
        WebhooksCmd::Delivery { webhook, delivery } => {
            let id = place.find(&api, &webhook).await?;
            let d = api.webhook_delivery(&id, &delivery).await?;
            if json {
                return style::print_json(&d);
            }
            print_delivery(&d);
            Ok(())
        }
        WebhooksCmd::Resend {
            webhook,
            delivery,
            failed,
            since,
        } => {
            let id = place.find(&api, &webhook).await?;
            if failed {
                let since = match since {
                    Some(s) => Some(style::now_ms() - duration(&s)?),
                    None => None,
                };
                let r = api
                    .resend_failed_webhook_deliveries(&id, &ResendFailedRequest { since })
                    .await?;
                if json {
                    return style::print_json(&r);
                }
                println!(
                    "{} again, in their turn.",
                    style::count(r.queued, "failed delivery waits", "failed deliveries wait")
                );
                return Ok(());
            }
            let delivery = delivery.context("Which delivery? Pass its id (whd_…), or --failed.")?;
            let d = api.resend_webhook_delivery(&id, &delivery).await?;
            if json {
                return style::print_json(&d);
            }
            println!("Resent {}: {}", d.id, outcome(&d));
            Ok(())
        }
        WebhooksCmd::Rm { webhook } => {
            let id = place.find(&api, &webhook).await?;
            api.delete_webhook(&id).await?;
            if json {
                return style::print_json(&serde_json::json!({ "ok": true }));
            }
            println!("Deleted {id}, and what went out to it.");
            Ok(())
        }
        WebhooksCmd::Events => {
            let catalog = api.webhook_events().await?;
            if json {
                return style::print_json(&catalog);
            }
            for g in &catalog.groups {
                println!("{}", style::bold(&g.label));
                for e in catalog.events.iter().filter(|e| e.group == g.id) {
                    println!("  {:<26} {}", e.kind, e.label);
                    println!("  {:<26} {}", "", style::dim(&e.description));
                }
                println!();
            }
            println!("{}", reach_words(&catalog.reach));
            Ok(())
        }
        WebhooksCmd::Verify { .. } => Ok(()),
    }
}

/// Whose webhooks: a project's, or the workspace's.
struct Place {
    workspace: String,
    project: Option<String>,
    name: String,
}

impl Place {
    async fn of(api: &Api, args: &WebhooksArgs) -> Result<Self> {
        if let Some(p) = &args.project {
            let project = api.project(p).await?;
            return Ok(Self {
                workspace: project
                    .workspace
                    .as_ref()
                    .map(|w| w.id.0.clone())
                    .unwrap_or_default(),
                project: Some(project.id.0.clone()),
                name: project.name,
            });
        }
        let w = super::workspaces::resolve(api, args.workspace.clone()).await?;
        Ok(Self {
            workspace: w.id.0.clone(),
            project: None,
            name: w.name,
        })
    }

    /// Where webhook `id` is: its project, or its workspace.
    async fn of_webhook(api: &Api, id: &str) -> Result<Self> {
        let w = api.get_webhook(id).await?;
        if let Some(p) = w.project {
            return Ok(Self {
                workspace: w.workspace_id.0,
                project: Some(p.id),
                name: p.name,
            });
        }
        let ws = api.workspace(&w.workspace_id.0).await?;
        Ok(Self {
            workspace: ws.id.0,
            project: None,
            name: ws.name,
        })
    }

    async fn list(&self, api: &Api) -> Result<fctl_proto::hooks::WebhookList> {
        Ok(match &self.project {
            Some(p) => api.project_webhooks(p).await?,
            None => api.workspace_webhooks(&self.workspace).await?,
        })
    }

    /// A webhook's id, from its id, its name or its host.
    async fn find(&self, api: &Api, given: &str) -> Result<String> {
        if fctl_proto::WebhookId::is_valid(given) {
            return Ok(given.to_owned());
        }
        let list = self.list(api).await?;
        let found: Vec<&Webhook> = list
            .webhooks
            .iter()
            .filter(|w| {
                w.name
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(given))
                    || rules::host_of(&w.url).eq_ignore_ascii_case(given)
            })
            .collect();
        match found.as_slice() {
            [one] => Ok(one.id.0.clone()),
            [] => bail!(
                "{} has no webhook called {given}: fctl webhooks lists them.",
                self.name
            ),
            _ => bail!("Several webhooks are called {given}: give its id (whk_…)."),
        }
    }
}

/// The events `--events` asks for: none given, unchanged (`None`); `all`,
/// every one (empty).
fn events(given: &[String]) -> Result<Option<Vec<String>>> {
    if given.is_empty() {
        return Ok(None);
    }
    let mut out = Vec::new();
    for g in given.iter().map(|g| g.trim()).filter(|g| !g.is_empty()) {
        if g == "all" || g == "*" {
            return Ok(Some(Vec::new()));
        }
        if let Some(group) = g.strip_suffix(".*") {
            let of: Vec<Kind> = Kind::ALL
                .into_iter()
                .filter(|k| k.as_str().split('.').next() == Some(group))
                .collect();
            if of.is_empty() {
                bail!(
                    "No events are called {g}: use session.*, pull_request.*, review.* or schedule.*."
                );
            }
            out.extend(of.into_iter().map(|k| k.as_str().to_owned()));
            continue;
        }
        match Kind::parse(g) {
            Some(k) => out.push(k.as_str().to_owned()),
            None => bail!("fctl has no event called {g}: fctl webhooks events lists them."),
        }
    }
    Ok(Some(
        rules::check_events(&out).map_err(|e| anyhow::anyhow!(e.message))?,
    ))
}

fn started_by(given: &[String]) -> Result<Option<Vec<WebhookOrigin>>> {
    if given.is_empty() {
        return Ok(None);
    }
    let mut out = Vec::new();
    for g in given.iter().map(|g| g.trim().to_ascii_lowercase()) {
        let o = match g.as_str() {
            "all" | "any" | "*" => return Ok(Some(Vec::new())),
            "person" | "people" | "persons" | "tools" => WebhookOrigin::Person,
            "schedule" | "schedules" => WebhookOrigin::Schedule,
            "autopilot" | "autopilots" => WebhookOrigin::Autopilot,
            "ticket" | "tickets" | "linear" => WebhookOrigin::Ticket,
            _ => bail!(
                "Sessions start from people, schedules, an autopilot or tickets: {g} isn't one."
            ),
        };
        if !out.contains(&o) {
            out.push(o);
        }
    }
    Ok(Some(out))
}

/// The ids of `--projects`: none given, unchanged; `all`, every project.
async fn projects(api: &Api, place: &Place, given: &[String]) -> Result<Option<Vec<ProjectId>>> {
    if given.is_empty() {
        return Ok(None);
    }
    if given.iter().any(|g| g == "all" || g == "*") {
        return Ok(Some(Vec::new()));
    }
    if place.project.is_some() {
        bail!("A project's webhook gets its own project's events: leave --projects out.");
    }
    let theirs = api.workspace_projects(&place.workspace).await?.projects;
    let mut out = Vec::new();
    for g in given.iter().map(|g| g.trim()) {
        let p = theirs
            .iter()
            .find(|p| p.id.0 == g || p.slug.eq_ignore_ascii_case(g))
            .with_context(|| format!("{} has no project {g}.", place.name))?;
        if !out.contains(&p.id) {
            out.push(p.id.clone());
        }
    }
    Ok(Some(out))
}

/// The names of the projects webhooks are narrowed to, by id.
async fn project_names(
    api: &Api,
    place: &Place,
    webhooks: &[Webhook],
) -> std::collections::HashMap<String, String> {
    if webhooks.iter().all(|w| w.projects.is_empty()) {
        return Default::default();
    }
    api.workspace_projects(&place.workspace)
        .await
        .map(|l| l.projects.into_iter().map(|p| (p.id.0, p.name)).collect())
        .unwrap_or_default()
}

fn title(w: &Webhook) -> String {
    w.name.clone().unwrap_or_else(|| rules::host_of(&w.url))
}

fn describe(w: &Webhook, names: &std::collections::HashMap<String, String>) -> String {
    let projects: Vec<String> = w
        .projects
        .iter()
        .map(|p| names.get(&p.0).cloned().unwrap_or_else(|| p.0.clone()))
        .collect();
    rules::describe(&w.url, &w.events, &projects, &w.started_by)
}

fn print_webhook(w: &Webhook, names: &std::collections::HashMap<String, String>) {
    let failing = w.health.as_ref().and_then(|h| h.failing_since);
    let (dot, state) = if !w.enabled {
        (style::dim("○"), style::dim(" (off)"))
    } else if failing.is_some() {
        (style::amber("●"), style::amber(" (failing)"))
    } else {
        (style::accent("●"), String::new())
    };
    println!(
        "{dot} {}{state}  {}",
        style::bold(&style::terminal_safe(&title(w))),
        style::dim(w.id.as_str())
    );
    println!("  {}", style::terminal_safe(&describe(w, names)));
    let mut facts = vec![match &w.project {
        Some(p) => format!("{}'s own", p.name),
        None => "the workspace's".to_owned(),
    }];
    facts.push(style::terminal_safe(&w.url).into_owned());
    if let Some(h) = &w.health {
        facts.push(format!(
            "{} delivered, {} failed in the last day",
            h.delivered, h.failed
        ));
        if h.pending > 0 {
            facts.push(format!("{} waiting", h.pending));
        }
        if let Some(last) = &h.last {
            facts.push(format!(
                "last {}: {}",
                style::ago(last.at),
                attempt_words(last)
            ));
        }
    }
    println!("  {}", style::dim(&facts.join(" · ")));
    if let Some(why) = &w.disabled_reason {
        println!("  {}", style::amber(&style::terminal_safe(why)));
    } else if let Some(since) = failing {
        println!(
            "  {}",
            style::amber(&format!(
                "fctl's attempts have all failed since {}: it turns it off after 3 days of that.",
                style::ago(since)
            ))
        );
    }
    if let Some(h) = &w.health
        && h.dropped > 0
    {
        println!(
            "  {}",
            style::amber(&format!(
                "{} dropped since it last took one: 1,000 deliveries waited for it already, so {} won't be sent.",
                style::count(h.dropped, "event was", "events were"),
                if h.dropped == 1 { "it" } else { "they" }
            ))
        );
    }
}

/// "HTTP 200 in 182 ms", "No answer within 10 seconds…".
fn attempt_words(a: &WebhookAttempt) -> String {
    match (a.status, &a.error) {
        (Some(s), None) => format!("HTTP {s} in {} ms", a.ms),
        (Some(_), Some(e)) => format!("{e} in {} ms", a.ms),
        (None, Some(e)) => e.clone(),
        (None, None) => "no attempt".into(),
    }
}

/// How a delivery went, in a few words.
fn outcome(d: &WebhookDelivery) -> String {
    let last = d.last.as_ref().map(attempt_words).unwrap_or_default();
    match d.state {
        WebhookDeliveryState::Delivered => style::accent(&format!("delivered, {last}")),
        WebhookDeliveryState::Pending => {
            let next = d
                .next_at
                .map(|at| format!("; fctl tries again {}", style::until(at)))
                .unwrap_or_default();
            style::amber(&format!("{last}{next}"))
        }
        _ => style::red(&format!("failed: {last}")),
    }
}

fn print_delivery_line(d: &WebhookDelivery) {
    let mark = match d.state {
        WebhookDeliveryState::Delivered => style::accent("✓"),
        WebhookDeliveryState::Pending => style::amber("…"),
        _ => style::red("✗"),
    };
    let status = d
        .last
        .as_ref()
        .and_then(|a| a.status)
        .map_or_else(|| "---".to_owned(), |s| s.to_string());
    let test = if d.test {
        style::dim(" (test)")
    } else {
        String::new()
    };
    println!(
        "{mark} {status} {:<25} {}{test}  {}  {}",
        d.kind,
        style::terminal_safe(&d.gist),
        style::dim(&style::ago(d.created_at)),
        style::dim(d.id.as_str())
    );
}

fn print_delivery(d: &WebhookDelivery) {
    println!(
        "{} · {} · {}",
        style::bold(d.id.as_str()),
        d.kind,
        outcome(d)
    );
    println!("{}", style::terminal_safe(&d.gist));
    if let Some(r) = &d.request {
        println!("\nPOST {}", style::terminal_safe(&r.url));
        for h in &r.headers {
            println!(
                "{}",
                style::dim(&format!("{}: {}", h.name, style::terminal_safe(&h.value)))
            );
        }
        let body = serde_json::from_str::<serde_json::Value>(&r.body)
            .ok()
            .and_then(|v| serde_json::to_string_pretty(&v).ok())
            .unwrap_or_else(|| r.body.clone());
        println!("\n{}", style::terminal_safe(&body));
    }
    if !d.history.is_empty() {
        println!("\n{}", style::bold("Attempts, newest first"));
        for a in &d.history {
            let by =
                a.by.as_ref()
                    .map(|u| format!(" (by {})", u.name))
                    .unwrap_or_default();
            println!(
                "  {}  {}{by}",
                style::dim(&style::ago(a.at)),
                attempt_words(a)
            );
        }
    }
    if let Some(resp) = d.last.as_ref().and_then(|a| a.response.as_ref()) {
        println!("\n{}", style::bold("The last answer"));
        for h in &resp.headers {
            println!(
                "{}",
                style::dim(&format!("{}: {}", h.name, style::terminal_safe(&h.value)))
            );
        }
        if let Some(body) = &resp.body {
            println!("\n{}", style::terminal_safe(body));
            if resp.truncated {
                println!("{}", style::dim("(its first 4 KB)"));
            }
        }
    }
}

fn first_lines(text: &str, n: usize) -> String {
    text.lines().take(n).collect::<Vec<_>>().join("\n")
}

/// "6h", "2d", "30m" in milliseconds, at most 30 days.
fn duration(s: &str) -> Result<i64> {
    let s = s.trim();
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: i64 = n
        .parse()
        .with_context(|| format!("{s:?} isn't a duration: use 30m, 6h or 2d"))?;
    let per: i64 = match unit {
        "m" => 60_000,
        "h" | "" => 3_600_000,
        "d" => 86_400_000,
        _ => bail!("{s:?} isn't a duration: use 30m, 6h or 2d"),
    };
    // Past 30 days is all of the log; past what fits, it isn't a duration.
    let ms = n
        .checked_mul(per)
        .with_context(|| format!("{s:?} is longer than any log keeps: use at most 30d"))?;
    Ok(ms.min(rules::KEPT_MS))
}

/// What `fctl webhooks verify` says of a request: whether the secret
/// signed it, checked first so that a saved request's answer says so
/// whatever its age, then, unless `any_age`, whether its timestamp is
/// within 5 minutes of `now_s`, as a receiver checks it (`rules::verify`).
fn verdict(
    secret: &str,
    id: &str,
    timestamp: &str,
    body: &[u8],
    signature: &str,
    now_s: i64,
    any_age: bool,
) -> std::result::Result<String, String> {
    let signed_at: i64 = match timestamp.trim().parse() {
        Ok(t) => t,
        Err(_) => return Err("webhook-timestamp isn't a number of seconds.".into()),
    };
    match rules::verify(secret, id, timestamp, body, signature, signed_at) {
        Ok(()) | Err(rules::Unverified::TooOld) => {}
        Err(rules::Unverified::Secret) => {
            return Err("That isn't a webhook secret: it starts with whsec_.".into());
        }
        Err(rules::Unverified::Timestamp) => {
            return Err("webhook-timestamp isn't a number of seconds.".into());
        }
        Err(rules::Unverified::Signature) => {
            return Err(
                "No signature matches: check the secret, and that the body is byte for byte as it came."
                    .into(),
            );
        }
    }
    const SIGNED: &str = "The signature is fctl's, made with this secret";
    if now_s.abs_diff(signed_at) <= rules::TOLERANCE_S.unsigned_abs() {
        return Ok(format!("{SIGNED}."));
    }
    let when = if signed_at < now_s {
        format!("signed {}", style::ago_from(signed_at * 1000, now_s * 1000))
    } else {
        "dated ahead of this machine's clock".to_owned()
    };
    if any_age {
        Ok(format!("{SIGNED}, {when}."))
    } else {
        Err(format!(
            "{SIGNED}, but {when}: more than 5 minutes from this machine's clock, so a receiver refuses it as a possible replay. --any-age checks a saved request's signature alone."
        ))
    }
}

/// Where the instance lets webhooks post, in a sentence.
fn reach_words(reach: &fctl_proto::hooks::WebhookReach) -> String {
    let scheme = if reach.http {
        "https:// and http://"
    } else {
        "https://"
    };
    if reach.private.iter().any(|p| p == "*") {
        return "Webhooks post to any address, http:// too: this is a development instance.".into();
    }
    if reach.private.is_empty() {
        return format!("Webhooks post to {scheme} addresses on the public internet.");
    }
    format!(
        "Webhooks post to {scheme} addresses on the public internet, and on this instance's network to {}.",
        reach.private.join(", ")
    )
}

impl WebhooksCmd {
    /// The webhook it's about, where it's about one.
    fn webhook(&self) -> Option<&str> {
        match self {
            Self::Show { webhook, .. }
            | Self::Edit { webhook, .. }
            | Self::Secret { webhook, .. }
            | Self::Test { webhook, .. }
            | Self::Deliveries { webhook, .. }
            | Self::Delivery { webhook, .. }
            | Self::Resend { webhook, .. }
            | Self::Rm { webhook, .. } => Some(webhook),
            Self::List | Self::Add { .. } | Self::Events | Self::Verify { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_says_whether_the_secret_signed_it_and_whether_a_receiver_takes_it() {
        let secret = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";
        let key = rules::key(secret).unwrap_or_default();
        let body = br#"{"type":"session.started"}"#;
        let at = 1_763_400_000;
        let sig = rules::signatures(&[key], "evt_1", at, body);
        let say = |now: i64, any_age: bool| {
            verdict(secret, "evt_1", &at.to_string(), body, &sig, now, any_age)
        };
        assert_eq!(
            say(at + 60, false),
            Ok("The signature is fctl's, made with this secret.".into())
        );
        // A saved one: signed with this secret, which it says, but too old
        // for a receiver unless --any-age.
        let old = say(at + 2 * 86_400, false).err().unwrap_or_default();
        assert!(
            old.starts_with("The signature is fctl's, made with this secret, but signed 2d ago")
        );
        assert!(old.contains("--any-age"));
        assert_eq!(
            say(at + 2 * 86_400, true),
            Ok("The signature is fctl's, made with this secret, signed 2d ago.".into())
        );
        assert!(
            say(at - 3_600, false)
                .err()
                .unwrap_or_default()
                .contains("dated ahead")
        );
        // Another secret's is refused, however old.
        let other = verdict(
            "whsec_b3RoZXI=",
            "evt_1",
            &at.to_string(),
            body,
            &sig,
            at + 86_400,
            true,
        );
        assert!(
            other
                .err()
                .unwrap_or_default()
                .starts_with("No signature matches")
        );
        assert!(verdict(secret, "evt_1", "soon", body, &sig, at, true).is_err());
    }

    #[test]
    fn events_say_where_the_instance_lets_webhooks_post() {
        use fctl_proto::hooks::WebhookReach;
        assert_eq!(
            reach_words(&WebhookReach::default()),
            "Webhooks post to https:// addresses on the public internet."
        );
        assert_eq!(
            reach_words(&WebhookReach {
                http: true,
                private: vec!["10.0.0.0/8".into(), "*.corp.internal".into()],
            }),
            "Webhooks post to https:// and http:// addresses on the public internet, and on this instance's network to 10.0.0.0/8, *.corp.internal."
        );
    }

    #[test]
    fn a_command_given_a_webhook_names_it() {
        use clap::Parser;
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: WebhooksArgs,
        }
        let cmd = |argv: &[&str]| {
            Cli::try_parse_from(argv)
                .ok()
                .and_then(|c| c.args.cmd)
                .and_then(|c| c.webhook().map(str::to_owned))
        };
        assert_eq!(
            cmd(&[
                "fctl",
                "edit",
                "whk_01KCTEST",
                "--url",
                "https://example.com/v2"
            ])
            .as_deref(),
            Some("whk_01KCTEST")
        );
        assert_eq!(
            cmd(&["fctl", "resend", "Slack", "--failed", "--since", "1h"]).as_deref(),
            Some("Slack")
        );
        assert_eq!(cmd(&["fctl", "list"]), None);
        assert_eq!(cmd(&["fctl", "events"]), None);
    }

    #[test]
    fn events_take_names_groups_and_all() {
        assert_eq!(events(&[]).ok(), Some(None));
        assert_eq!(events(&["all".into()]).ok(), Some(Some(vec![])));
        let reviews = events(&["review.*".into()])
            .ok()
            .flatten()
            .unwrap_or_default();
        assert_eq!(reviews.len(), 5);
        assert!(reviews.iter().all(|e| e.starts_with("review.")));
        assert_eq!(
            events(&["session.failed".into(), "session.started".into()]).ok(),
            Some(Some(vec![
                "session.started".into(),
                "session.failed".into()
            ]))
        );
        assert!(events(&["session.exploded".into()]).is_err());
        assert!(events(&["nope.*".into()]).is_err());
    }

    #[test]
    fn whom_sessions_start_from_reads_plainly() {
        assert_eq!(
            started_by(&["schedules".into(), "people".into()]).ok(),
            Some(Some(vec![WebhookOrigin::Schedule, WebhookOrigin::Person]))
        );
        assert_eq!(started_by(&["all".into()]).ok(), Some(Some(vec![])));
        assert!(started_by(&["robots".into()]).is_err());
    }

    #[test]
    fn durations_are_minutes_hours_or_days() {
        assert_eq!(duration("30m").ok(), Some(1_800_000));
        assert_eq!(duration("6h").ok(), Some(21_600_000));
        assert_eq!(duration("90d").ok(), Some(rules::KEPT_MS));
        assert!(duration("soon").is_err());
        assert!(duration("1000000000000d").is_err());
    }
}
