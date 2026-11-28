//! What fctl posts in Slack, as Block Kit: the card a session keeps
//! current in its thread, the agent's questions with a button per answer,
//! its replies, what fctl asks of people only they see (link your account,
//! choose the project), notifications, the Home tab and the forms that
//! start sessions.
//!
//! Text an agent wrote goes through [`text::defuse`] (or Slack's own
//! escaping, in Slack's markup), so that it notifies nobody, and in
//! `verbatim` text objects, so that Slack doesn't make links or mentions of
//! plain words.

use super::text;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Slack's most blocks in a message.
pub const MAX_BLOCKS: usize = 50;
/// The most of a plan's tasks fctl shows.
pub const MAX_TASKS: usize = 20;
/// Button labels Slack shows in full.
pub const MAX_BUTTON: usize = 30;

/// Action ids fctl's buttons and menus carry, which the interactivity
/// address tells apart.
pub mod act {
    /// An answer to the agent's question: `fctl_answer_<n>`.
    pub const ANSWER: &str = "fctl_answer_";
    /// A project chosen for a request that waits for one.
    pub const PROJECT: &str = "fctl_project";
    /// "Remember for #channel", beside the choice.
    pub const REMEMBER: &str = "fctl_remember";
    /// Opens the session in fctl (a link: Slack still tells fctl).
    pub const OPEN: &str = "fctl_open";
    /// Opens the form that starts a session (the Home tab).
    pub const START: &str = "fctl_start";
    /// Links the person's Slack account (a link).
    pub const LINK: &str = "fctl_link";
    /// Forgets a request that waits.
    pub const CANCEL: &str = "fctl_cancel";
}

/// Callback ids of fctl's forms and shortcuts.
pub mod callback {
    /// The form Send to fctl… opens.
    pub const SEND: &str = "fctl_send";
    /// The form that starts a session from the Home tab, `/fctl` or the
    /// global shortcut.
    pub const START: &str = "fctl_start";
    /// The message shortcut, as the manifest names it.
    pub const SEND_SHORTCUT: &str = "fctl_send_message";
    /// The global shortcut.
    pub const START_SHORTCUT: &str = "fctl_start_session";
}

fn mrkdwn(text: &str) -> Value {
    json!({ "type": "mrkdwn", "text": text, "verbatim": true })
}

fn plain(text: &str) -> Value {
    json!({ "type": "plain_text", "text": text, "emoji": true })
}

fn section(text: &str) -> Value {
    json!({ "type": "section", "text": mrkdwn(&cap(text, text::MAX_SECTION)) })
}

fn context(text: &str) -> Value {
    json!({ "type": "context", "elements": [mrkdwn(&cap(text, 2000))] })
}

fn button(label: &str, action_id: &str) -> Value {
    json!({ "type": "button", "text": plain(&text::clip(label, 75)), "action_id": action_id })
}

fn link_button(label: &str, action_id: &str, url: &str) -> Value {
    let mut b = button(label, action_id);
    b["url"] = json!(url);
    b
}

/// Text cut to Slack's `max` characters.
fn cap(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let mut c: String = text.chars().take(max - 1).collect();
        c.push('…');
        c
    }
}

/// A time as Slack shows it to each reader in their own time zone, with
/// `fallback` (UTC) for what can't.
pub fn local_time(at_ms: i64, fallback: &str) -> String {
    format!(
        "<!date^{}^{{date_short_pretty}} at {{time}}|{}>",
        at_ms.div_euclid(1000),
        text::escape(fallback)
    )
}

// ───────────────────────────── the card ─────────────────────────────

/// How a session stands, in the words its card says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Status {
    /// It waits for a runner.
    Queued,
    /// Its sandbox starts.
    Starting,
    /// The agent works: what it does now, as its step says.
    Working {
        step: Option<String>,
    },
    /// It waits for an answer to the agent's question.
    Asking,
    /// It finished its turn and waits for a reply.
    Waiting,
    /// Its pull requests merged, and it asks nothing.
    Done,
    /// It paused on a Claude usage limit, until a time (milliseconds), with
    /// that time in UTC for what can't show it in the reader's zone.
    Paused {
        until: Option<i64>,
        until_utc: Option<String>,
    },
    Failed {
        reason: Option<String>,
    },
    /// Someone stopped it, or it stopped idle: a reply resumes it.
    Stopped,
}

impl Status {
    /// The emoji and words of its line.
    pub fn line(&self) -> String {
        match self {
            Self::Queued => ":hourglass_flowing_sand: Waiting for a runner".into(),
            Self::Starting => ":hourglass_flowing_sand: Starting its sandbox".into(),
            Self::Working { step } => format!(
                ":gear: {}",
                step.as_deref()
                    .map(|s| text::escape(&text::clip(s, 120)))
                    .unwrap_or_else(|| "Working".into())
            ),
            Self::Asking => ":raising_hand: Waiting for your answer".into(),
            Self::Waiting => ":speech_balloon: Waiting for a reply".into(),
            Self::Done => ":white_check_mark: Done".into(),
            Self::Paused { until, until_utc } => match until {
                Some(at) => format!(
                    ":double_vertical_bar: Paused until {}: its Claude subscription reached a usage limit",
                    local_time(*at, until_utc.as_deref().unwrap_or("later"))
                ),
                None => {
                    ":double_vertical_bar: Paused: its Claude subscription reached a usage limit"
                        .into()
                }
            },
            Self::Failed { reason } => match reason.as_deref().filter(|r| !r.trim().is_empty()) {
                Some(r) => format!(":x: Failed: {}", text::escape(&text::clip(r, 200))),
                None => ":x: Failed".into(),
            },
            Self::Stopped => ":black_square_for_stop: Stopped: reply here to carry on".into(),
        }
    }

    /// The words alone, for a notification's text.
    pub fn words(&self) -> String {
        let line = self.line();
        line.split_once(' ')
            .map_or(line.clone(), |(_, w)| w.to_owned())
            .replace("<!date^", "")
    }
}

/// A task of the agent's plan (Claude Code's todo list).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub title: String,
    /// `pending`, `in_progress` or `complete`.
    pub status: String,
}

/// The agent's todo list (`TodoWrite`'s `todos`) as a plan's tasks.
pub fn plan(todos: &Value) -> Vec<Task> {
    todos
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    let title = t
                        .get("content")
                        .or_else(|| t.get("activeForm"))
                        .and_then(Value::as_str)?;
                    let status = match t.get("status").and_then(Value::as_str) {
                        Some("completed") => "complete",
                        Some("in_progress") => "in_progress",
                        Some("cancelled" | "canceled") => return None,
                        _ => "pending",
                    };
                    Some(Task {
                        title: text::clip(title, 140),
                        status: status.into(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A pull request of the session, as its card shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pull {
    /// `acme/web#42`.
    pub label: String,
    pub url: String,
    pub title: Option<String>,
    /// `open`, `draft`, `queued`, `merged` or `closed`.
    pub state: Option<String>,
    /// Its checks: passed, failed, to finish.
    pub checks: Option<(u32, u32, u32)>,
}

impl Pull {
    fn line(&self) -> String {
        let title = self
            .title
            .as_deref()
            .map(|t| format!(" {}", text::escape(&text::clip(t, 90))))
            .unwrap_or_default();
        let state = match self.state.as_deref() {
            Some("merged") => " · :large_purple_circle: merged".to_owned(),
            Some("closed") => " · closed".to_owned(),
            Some("draft") => " · draft".to_owned(),
            Some("queued") => " · in the merge queue".to_owned(),
            _ => match self.checks {
                Some((_, failed, _)) if failed > 0 => format!(
                    " · :x: {failed} {} failed",
                    if failed == 1 { "check" } else { "checks" }
                ),
                Some((_, _, pending)) if pending > 0 => {
                    " · :large_yellow_circle: checks running".into()
                }
                Some((passed, _, _)) if passed > 0 => " · :white_check_mark: checks passed".into(),
                _ => String::new(),
            },
        };
        format!(
            ":arrow_heading_up: {}{title}{state}",
            text::link(&self.url, &self.label)
        )
    }
}

/// What a session's card shows: compact, kept current in place as the
/// session moves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub title: String,
    /// The session's page in fctl.
    pub url: String,
    /// `web#12`.
    pub reference: String,
    /// The workspace's name, which `reference`'s project is in.
    pub workspace: String,
    /// Whom it runs for.
    pub by: String,
    pub status: Status,
    #[serde(default)]
    pub plan: Vec<Task>,
    #[serde(default)]
    pub pulls: Vec<Pull>,
}

/// The card's blocks: its title and how it stands, who it runs for, the
/// agent's plan as Slack draws one, its pull requests with their checks,
/// and one button to open it. `plan_block`: Slack's own `plan` block, else
/// the plan as a list, for where Slack refuses it.
pub fn card(c: &Card, plan_block: bool) -> Value {
    let mut blocks = vec![
        section(&format!(
            "*{}*\n{}",
            text::link(&c.url, &text::clip(&c.title, 150)),
            c.status.line()
        )),
        context(&if c.workspace.is_empty() {
            format!(
                "{} · for {}",
                text::escape(&c.reference),
                text::escape(&c.by)
            )
        } else {
            format!(
                "{} · {} · for {}",
                text::escape(&c.workspace),
                text::escape(&c.reference),
                text::escape(&c.by)
            )
        }),
    ];
    if !c.plan.is_empty() {
        let done = c.plan.iter().filter(|t| t.status == "complete").count();
        let title = format!("{done} of {} done", c.plan.len());
        if plan_block {
            let tasks: Vec<Value> = c
                .plan
                .iter()
                .take(MAX_TASKS)
                .enumerate()
                .map(|(i, t)| {
                    json!({ "task_id": format!("t{i}"), "title": t.title, "status": t.status })
                })
                .collect();
            blocks.push(json!({ "type": "plan", "title": title, "tasks": tasks }));
        } else {
            let lines: Vec<String> = c
                .plan
                .iter()
                .take(MAX_TASKS)
                .map(|t| {
                    let mark = match t.status.as_str() {
                        "complete" => ":white_check_mark:",
                        "in_progress" => ":arrow_forward:",
                        _ => ":white_circle:",
                    };
                    format!("{mark} {}", text::escape(&t.title))
                })
                .collect();
            blocks.push(section(&format!("*{title}*\n{}", lines.join("\n"))));
        }
    }
    if !c.pulls.is_empty() {
        let lines: Vec<String> = c.pulls.iter().take(5).map(Pull::line).collect();
        blocks.push(section(&lines.join("\n")));
    }
    blocks.push(json!({
        "type": "actions",
        "elements": [link_button("Open in fctl", act::OPEN, &c.url)],
    }));
    Value::Array(blocks)
}

/// The card's text, for notifications and what can't show blocks.
pub fn card_text(c: &Card) -> String {
    format!("{}: {} ({})", c.reference, c.title, c.status.words())
}

// ───────────────────────────── replies and questions ─────────────────────────────

/// A message of the agent's (its turn's last), in a `markdown` block that
/// Slack draws as the agent wrote it, its mentions defused; `more` links
/// the rest where it was cut. And its text, for notifications.
pub fn reply(markdown: &str, session_url: &str) -> (Value, String) {
    let more = format!("[The rest is in fctl]({session_url})");
    let (body, _) = text::cut(markdown, text::MAX_REPLY, &more);
    let body = text::defuse(&body);
    let fallback = text::clip(&text::plain(&body, &|_| None), 300);
    (json!([{ "type": "markdown", "text": body }]), fallback)
}

/// The same reply in Slack's own markup, for where Slack refuses the
/// `markdown` block.
pub fn reply_mrkdwn(markdown: &str, session_url: &str) -> Value {
    let more = format!("[The rest is in fctl]({session_url})");
    let (body, _) = text::cut(markdown, text::MAX_REPLY - 200, &more);
    let m = text::mrkdwn(&body);
    let mut blocks: Vec<Value> = Vec::new();
    let mut chunk = String::new();
    for para in m.split("\n\n") {
        if chunk.chars().count() + para.chars().count() + 2 > text::MAX_SECTION && !chunk.is_empty()
        {
            blocks.push(section(&chunk));
            chunk.clear();
        }
        if !chunk.is_empty() {
            chunk.push_str("\n\n");
        }
        chunk.push_str(para);
    }
    if !chunk.is_empty() {
        blocks.push(section(&chunk));
    }
    Value::Array(blocks)
}

/// A question of the agent's (`fctl ask`), with a button for each answer:
/// `block_id` names the question, so that a click on an old one answers
/// nothing. Its text, for notifications.
pub fn question(question: &str, options: &[String], block_id: &str) -> (Value, String) {
    let mut blocks = vec![section(&format!(
        ":raising_hand: *The agent asks*\n{}",
        text::mrkdwn(question)
    ))];
    if !options.is_empty() {
        let buttons: Vec<Value> = options
            .iter()
            .take(6)
            .enumerate()
            .map(|(i, o)| {
                let mut b = button(o, &format!("{}{i}", act::ANSWER));
                b["value"] = json!(i.to_string());
                b
            })
            .collect();
        blocks.push(json!({ "type": "actions", "block_id": block_id, "elements": buttons }));
    }
    blocks.push(context(if options.is_empty() {
        "Reply in this thread to answer."
    } else {
        "Click an answer, or reply in this thread."
    }));
    (
        Value::Array(blocks),
        format!("The agent asks: {}", text::clip(question, 200)),
    )
}

/// The question once someone answered it: who, and what, in place of its
/// buttons.
pub fn answered(question: &str, by: &str, answer: &str) -> Value {
    json!([
        section(&format!(
            ":raising_hand: *The agent asked*\n{}",
            text::mrkdwn(question)
        )),
        context(&format!(
            "{} answered: *{}*",
            text::escape(by),
            text::escape(&text::clip(answer, 120))
        )),
    ])
}

/// A short message of fctl's in a thread: what it did, or why it couldn't.
pub fn note(emoji: &str, words: &str) -> Value {
    json!([section(&format!("{emoji} {words}"))])
}

// ───────────────────────────── what only one person sees ─────────────────────────────

/// Asks someone fctl doesn't know yet to link their Slack account: why,
/// and the link that does it, after which their request starts by itself.
pub fn link_account(link: &str, waiting: &str) -> Value {
    json!([
        section(&format!(
            ":wave: fctl doesn't know who you are yet. Link your Slack account to fctl once, and {waiting}: as you, on your Claude subscription."
        )),
        { "type": "actions", "elements": [{
            "type": "button", "text": plain("Link your Slack account"), "action_id": act::LINK,
            "url": link, "style": "primary"
        }] },
        context("Only you see this. fctl keeps your request for an hour, and reacts to it once it starts."),
    ])
}

/// A project to choose, by its id, with what it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub name: String,
    /// Its workspace, where projects of several are offered.
    pub workspace: Option<String>,
}

/// Asks which project should work on a request: a button each for a few,
/// a menu for more; with "Remember for #channel" for the workspace's admins
/// (`remember`: the channel's name).
pub fn choose_project(choices: &[Choice], remember: Option<&str>, thread: &str) -> Value {
    let label = |c: &Choice| match &c.workspace {
        Some(w) => format!("{} ({w})", c.name),
        None => c.name.clone(),
    };
    let mut blocks = vec![section(
        ":thinking_face: Which project should work on this?",
    )];
    if let Some(channel) = remember {
        blocks.push(json!({
            "type": "actions", "block_id": act::REMEMBER,
            "elements": [{
                "type": "checkboxes", "action_id": act::REMEMBER,
                "options": [{ "text": mrkdwn(&format!("Remember for #{}", text::escape(channel))), "value": "remember" }]
            }]
        }));
    }
    let picker = if choices.len() <= 5 {
        let mut buttons: Vec<Value> = choices
            .iter()
            .map(|c| {
                let mut b = button(&label(c), act::PROJECT);
                b["value"] = json!(format!("{thread}|{}", c.id));
                b
            })
            .collect();
        // Each button's action id is its own within a block.
        for (i, b) in buttons.iter_mut().enumerate() {
            b["action_id"] = json!(format!("{}_{i}", act::PROJECT));
        }
        buttons
    } else {
        vec![json!({
            "type": "static_select", "action_id": act::PROJECT, "placeholder": plain("Choose a project"),
            "options": choices.iter().take(100).map(|c| json!({
                "text": plain(&text::clip(&label(c), 75)), "value": format!("{thread}|{}", c.id)
            })).collect::<Vec<_>>()
        })]
    };
    let mut elements = picker;
    let mut cancel = button("Cancel", act::CANCEL);
    cancel["value"] = json!(thread);
    elements.push(cancel);
    blocks.push(json!({ "type": "actions", "block_id": "fctl_choose", "elements": elements }));
    blocks.push(context("Only you see this."));
    Value::Array(blocks)
}

/// A refusal or an answer only one person sees.
pub fn only_you(emoji: &str, words: &str) -> Value {
    json!([
        section(&format!("{emoji} {words}")),
        context("Only you see this.")
    ])
}

// ───────────────────────────── notifications ─────────────────────────────

/// A notification in someone's direct messages with fctl's app: what
/// happened to a session they follow, with the agent's question and its
/// answers where it asks (`question`: its text, answers and block id).
pub fn notification(
    headline: &str,
    detail: Option<&str>,
    session_url: &str,
    question: Option<(&str, &[String], &str)>,
) -> Value {
    let mut blocks = vec![section(headline)];
    match question {
        Some((q, options, block_id)) => {
            blocks.push(section(&format!(
                "> {}",
                text::mrkdwn(q).replace('\n', "\n> ")
            )));
            let mut elements: Vec<Value> = options
                .iter()
                .take(5)
                .enumerate()
                .map(|(i, o)| {
                    let mut b = button(o, &format!("{}{i}", act::ANSWER));
                    b["value"] = json!(i.to_string());
                    b
                })
                .collect();
            elements.push(link_button("Open in fctl", act::OPEN, session_url));
            blocks.push(json!({ "type": "actions", "block_id": block_id, "elements": elements }));
        }
        None => {
            if let Some(d) = detail.filter(|d| !d.trim().is_empty()) {
                blocks.push(section(&format!("> {}", text::escape(&text::clip(d, 600)))));
            }
            blocks.push(json!({ "type": "actions", "elements": [link_button("Open in fctl", act::OPEN, session_url)] }));
        }
    }
    Value::Array(blocks)
}

// ───────────────────────────── the Home tab ─────────────────────────────

/// A session in the Home tab's lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomeSession {
    pub reference: String,
    pub title: String,
    pub url: String,
    /// What it waits for, or does.
    pub status: String,
}

/// What fctl's Home tab shows someone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Home {
    /// Their name in fctl, once linked; else the link that links them.
    pub name: Option<String>,
    pub link: Option<String>,
    pub needs_you: Vec<HomeSession>,
    pub running: Vec<HomeSession>,
    /// Pull requests: their label, title and address.
    pub pulls: Vec<(String, String, String)>,
    /// fctl's address, for "Open fctl".
    pub app_url: String,
    /// Whether they may start sessions from here.
    pub can_start: bool,
}

/// The Home tab's view.
pub fn home(h: &Home) -> Value {
    let mut blocks = vec![json!({ "type": "header", "text": plain("fctl") })];
    match (&h.name, &h.link) {
        (Some(name), _) => {
            let mut row = vec![link_button("Open fctl", act::OPEN, &h.app_url)];
            if h.can_start {
                let mut start = button("Start a session", act::START);
                start["style"] = json!("primary");
                row.insert(0, start);
            }
            blocks.push(section(&format!(
                "Hi {}. Mention fctl in a channel, or write to it in Messages, and an agent starts on it as you. Its card in the thread says how it goes.",
                text::escape(name)
            )));
            blocks.push(json!({ "type": "actions", "elements": row }));
        }
        (None, Some(link)) => {
            blocks.push(section(
                "fctl runs Claude agents on your projects. Link your Slack account once, and what you ask fctl here runs as you, on your Claude subscription.",
            ));
            blocks.push(json!({ "type": "actions", "elements": [{
                "type": "button", "text": plain("Link your Slack account"), "action_id": act::LINK,
                "url": link, "style": "primary"
            }] }));
            return Value::Array(blocks);
        }
        (None, None) => {
            blocks.push(section("fctl isn't connected to an fctl workspace here anymore: one of its admins connects it again."));
            return Value::Array(blocks);
        }
    }
    let list = |blocks: &mut Vec<Value>, heading: &str, empty: &str, items: &[HomeSession]| {
        blocks.push(json!({ "type": "divider" }));
        blocks.push(json!({ "type": "header", "text": plain(heading) }));
        if items.is_empty() {
            blocks.push(context(empty));
        }
        for s in items.iter().take(10) {
            blocks.push(json!({
                "type": "section",
                "text": mrkdwn(&format!(
                    "*{}* {}\n{}",
                    text::link(&s.url, &s.reference),
                    text::escape(&text::clip(&s.title, 120)),
                    text::escape(&s.status)
                )),
                "accessory": link_button("Open", act::OPEN, &s.url),
            }));
        }
    };
    list(
        &mut blocks,
        "Needs you",
        "Nothing waits for you.",
        &h.needs_you,
    );
    list(
        &mut blocks,
        "Running",
        "No session of yours runs now.",
        &h.running,
    );
    blocks.push(json!({ "type": "divider" }));
    blocks.push(json!({ "type": "header", "text": plain("Recent pull requests") }));
    if h.pulls.is_empty() {
        blocks.push(context("Your sessions opened none lately."));
    } else {
        let lines: Vec<String> = h
            .pulls
            .iter()
            .take(10)
            .map(|(label, title, url)| {
                format!(
                    ":arrow_heading_up: {} {}",
                    text::link(url, label),
                    text::escape(&text::clip(title, 100))
                )
            })
            .collect();
        blocks.push(section(&lines.join("\n")));
    }
    Value::Array(blocks)
}

// ───────────────────────────── forms ─────────────────────────────

fn project_input(choices: &[Choice], chosen: Option<&str>) -> Value {
    let options: Vec<Value> = choices
        .iter()
        .take(100)
        .map(|c| {
            let label = match &c.workspace {
                Some(w) => format!("{} ({w})", c.name),
                None => c.name.clone(),
            };
            json!({ "text": plain(&text::clip(&label, 75)), "value": c.id })
        })
        .collect();
    let mut select = json!({
        "type": "static_select", "action_id": "project", "placeholder": plain("Choose a project"),
        "options": options
    });
    let initial = chosen
        .and_then(|id| choices.iter().position(|c| c.id == id))
        .or(if choices.len() == 1 { Some(0) } else { None });
    if let Some(i) = initial
        && let Some(o) = select["options"].get(i).cloned()
    {
        select["initial_option"] = o;
    }
    json!({ "type": "input", "block_id": "project", "label": plain("Project"), "element": select })
}

fn task_input(initial: &str, hint: &str) -> Value {
    let mut input = json!({
        "type": "plain_text_input", "action_id": "task", "multiline": true,
        "placeholder": plain("What should the agent do?")
    });
    if !initial.is_empty() {
        input["initial_value"] = json!(text::clip(initial, 3000));
    }
    json!({ "type": "input", "block_id": "task", "label": plain("Task"), "element": input, "hint": plain(hint) })
}

/// The form that starts a session: a project and a task. `metadata` comes
/// back with it.
pub fn start_form(choices: &[Choice], chosen: Option<&str>, task: &str, metadata: &str) -> Value {
    json!({
        "type": "modal", "callback_id": callback::START,
        "title": plain("Start a session"), "submit": plain("Start"), "close": plain("Cancel"),
        "private_metadata": metadata,
        "blocks": [
            project_input(choices, chosen),
            task_input(task, "It runs as you, on your Claude subscription. Its card comes to your messages with fctl."),
        ]
    })
}

/// The form Send to fctl… opens on a message: the message, a project, a
/// task, and whether fctl posts in the message's thread. `metadata` comes
/// back with it.
pub fn send_form(
    choices: &[Choice],
    chosen: Option<&str>,
    quoted_by: &str,
    quoted: &str,
    can_post: bool,
    metadata: &str,
) -> Value {
    let mut blocks = vec![
        context(&format!("Message from {}", text::escape(quoted_by))),
        section(&format!(
            "> {}",
            text::escape(&text::clip(quoted, 600)).replace('\n', "\n> ")
        )),
        project_input(choices, chosen),
        task_input(
            "",
            "The message and its thread go with it as information. The session runs as you, on your Claude subscription.",
        ),
    ];
    if can_post {
        blocks.push(json!({
            "type": "input", "block_id": "updates", "optional": true, "label": plain("Updates"),
            "element": {
                "type": "checkboxes", "action_id": "updates",
                "options": [{ "text": plain("Post its card and replies in this thread"), "value": "updates" }],
                "initial_options": [{ "text": plain("Post its card and replies in this thread"), "value": "updates" }]
            }
        }));
    } else {
        blocks.push(context(
            "fctl isn't in this channel, so the session's card and replies come to your messages with fctl.",
        ));
    }
    json!({
        "type": "modal", "callback_id": callback::SEND,
        "title": plain("Send to fctl"), "submit": plain("Start"), "close": plain("Cancel"),
        "private_metadata": metadata,
        "blocks": blocks
    })
}

// ───────────────────────────── project events ─────────────────────────────

/// A project's event as its channel shows it (`slack::SlackEvents`): what
/// happened, in a line, and a button to the session.
pub fn event(emoji: &str, headline: &str, detail: Option<&str>, url: &str) -> Value {
    let mut blocks = vec![section(&format!("{emoji} {headline}"))];
    if let Some(d) = detail.filter(|d| !d.trim().is_empty()) {
        blocks.push(context(&text::escape(&text::clip(d, 300))));
    }
    blocks.push(
        json!({ "type": "actions", "elements": [link_button("Open in fctl", act::OPEN, url)] }),
    );
    Value::Array(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a test expects to be there: it fails the test where it isn't.
    trait Must<T> {
        fn must(self) -> T;
    }

    impl<T, E: std::fmt::Debug> Must<T> for Result<T, E> {
        fn must(self) -> T {
            match self {
                Ok(v) => v,
                Err(e) => panic!("{e:?}"),
            }
        }
    }

    impl<T> Must<T> for Option<T> {
        fn must(self) -> T {
            match self {
                Some(v) => v,
                None => panic!("nothing there"),
            }
        }
    }

    fn sample() -> Card {
        Card {
            title: "Fix the flaky login test".into(),
            url: "https://fctl.acme.dev/s/ses_1".into(),
            reference: "web#12".into(),
            workspace: "Acme".into(),
            by: "Ada Lovelace".into(),
            status: Status::Working {
                step: Some("Running the tests".into()),
            },
            plan: plan(&json!([
                { "content": "Reproduce it", "status": "completed" },
                { "content": "Fix the race", "status": "in_progress" },
                { "content": "Add a test", "status": "pending" },
                { "content": "Dropped", "status": "cancelled" }
            ])),
            pulls: vec![Pull {
                label: "acme/web#42".into(),
                url: "https://github.com/acme/web/pull/42".into(),
                title: Some("Fix the login race".into()),
                state: Some("open".into()),
                checks: Some((11, 1, 0)),
            }],
        }
    }

    #[test]
    fn the_card_says_how_the_session_stands_in_a_few_blocks() {
        let c = sample();
        let b = card(&c, true);
        let blocks = b.as_array().must();
        assert_eq!(blocks.len(), 5);
        let head = blocks[0]["text"]["text"].as_str().must();
        assert!(head.starts_with(
            "*<https://fctl.acme.dev/s/ses_1|Fix the flaky login test>*\n:gear: Running the tests"
        ));
        assert_eq!(blocks[0]["text"]["verbatim"], true);
        assert_eq!(blocks[2]["type"], "plan");
        assert_eq!(blocks[2]["title"], "1 of 3 done");
        assert_eq!(blocks[2]["tasks"][1]["status"], "in_progress");
        let pr = blocks[3]["text"]["text"].as_str().must();
        assert!(pr.contains("<https://github.com/acme/web/pull/42|acme/web#42> Fix the login race · :x: 1 check failed"));
        assert_eq!(
            blocks[4]["elements"][0]["url"],
            "https://fctl.acme.dev/s/ses_1"
        );
        // Without Slack's plan block, a list.
        let listed = card(&c, false);
        assert!(
            listed[2]["text"]["text"]
                .as_str()
                .must()
                .contains(":arrow_forward: Fix the race")
        );
        assert_eq!(
            card_text(&c),
            "web#12: Fix the flaky login test (Running the tests)"
        );
    }

    #[test]
    fn statuses_read_as_words() {
        assert_eq!(Status::Asking.words(), "Waiting for your answer");
        assert!(
            Status::Paused {
                until: Some(1_700_000_000_000),
                until_utc: Some("14:13 UTC".into())
            }
            .line()
            .contains("<!date^1700000000^{date_short_pretty} at {time}|14:13 UTC>")
        );
        assert_eq!(
            Status::Failed {
                reason: Some("The runner <went> away".into())
            }
            .line(),
            ":x: Failed: The runner &lt;went&gt; away"
        );
    }

    #[test]
    fn replies_are_markdown_without_mentions_and_cut_past_slacks_limits() {
        let (b, text) = reply("Done: see **#42**. <!channel> @here", "https://x/s/1");
        let md = b[0]["text"].as_str().must();
        assert_eq!(b[0]["type"], "markdown");
        assert!(!md.contains("<!channel>") && !md.contains("@here"));
        assert!(text.starts_with("Done: see **#42**."));
        let long = "word ".repeat(2000);
        let (b, _) = reply(&long, "https://x/s/1");
        let md = b[0]["text"].as_str().must();
        assert!(md.chars().count() <= text::MAX_REPLY + 10);
        assert!(md.ends_with("[The rest is in fctl](https://x/s/1)"));
        let m = reply_mrkdwn("**Done**", "https://x/s/1");
        assert_eq!(m[0]["text"]["text"], "*Done*");
    }

    #[test]
    fn questions_carry_a_button_per_answer() {
        let (b, text) = question(
            "Ship it to production?",
            &["Yes".into(), "Not yet".into()],
            "fctl_q:ses_1:1700",
        );
        assert_eq!(b[1]["block_id"], "fctl_q:ses_1:1700");
        assert_eq!(b[1]["elements"][1]["action_id"], "fctl_answer_1");
        assert_eq!(b[1]["elements"][1]["value"], "1");
        assert_eq!(text, "The agent asks: Ship it to production?");
        let a = answered("Ship it?", "Ada Lovelace", "Yes");
        assert_eq!(a[1]["elements"][0]["text"], "Ada Lovelace answered: *Yes*");
    }

    #[test]
    fn choosing_a_project_offers_buttons_or_a_menu() {
        let few: Vec<Choice> = (0..3)
            .map(|i| Choice {
                id: format!("prj_{i}"),
                name: format!("P{i}"),
                workspace: None,
            })
            .collect();
        let b = choose_project(&few, Some("web-team"), "T1:C1:1.1");
        assert_eq!(b[1]["block_id"], act::REMEMBER);
        let buttons = b[2]["elements"].as_array().must();
        assert_eq!(buttons.len(), 4);
        assert_eq!(buttons[0]["value"], "T1:C1:1.1|prj_0");
        assert_eq!(buttons[3]["action_id"], act::CANCEL);
        let many: Vec<Choice> = (0..8)
            .map(|i| Choice {
                id: format!("prj_{i}"),
                name: format!("P{i}"),
                workspace: Some("Acme".into()),
            })
            .collect();
        let b = choose_project(&many, None, "T1:C1:1.1");
        assert_eq!(b[1]["elements"][0]["type"], "static_select");
        assert_eq!(
            b[1]["elements"][0]["options"][7]["text"]["text"],
            "P7 (Acme)"
        );
    }

    #[test]
    fn forms_preselect_the_only_project() {
        let one = vec![Choice {
            id: "prj_1".into(),
            name: "Web".into(),
            workspace: None,
        }];
        let f = start_form(&one, None, "", "{}");
        assert_eq!(
            f["blocks"][0]["element"]["initial_option"]["value"],
            "prj_1"
        );
        assert_eq!(f["callback_id"], callback::START);
        let s = send_form(&one, None, "Grace Hopper", "The deploy broke", true, "{}");
        assert_eq!(s["blocks"][4]["block_id"], "updates");
        let s = send_form(&one, None, "Grace Hopper", "The deploy broke", false, "{}");
        assert!(
            s["blocks"][4]["elements"][0]["text"]
                .as_str()
                .must()
                .contains("isn't in this channel")
        );
    }

    #[test]
    fn notifications_carry_the_question_or_what_happened() {
        let options = vec!["Yes".to_owned(), "Not yet".to_owned()];
        let n = notification(
            ":raising_hand: *web#12* asks you",
            None,
            "https://x/s/1",
            Some(("Ship it?", &options, "fctl_q:ses_1:5")),
        );
        assert_eq!(n[2]["block_id"], "fctl_q:ses_1:5");
        assert_eq!(n[2]["elements"][1]["action_id"], "fctl_answer_1");
        // The last button opens the session.
        assert_eq!(n[2]["elements"][2]["url"], "https://x/s/1");
        let w = notification(
            ":speech_balloon: waits",
            Some("Done <!here>"),
            "https://x/s/1",
            None,
        );
        assert_eq!(w[1]["text"]["text"], "> Done &lt;!here&gt;");
        let e = event(
            ":eyes:",
            "web#12 is ready for review",
            Some("Fix it"),
            "https://x/s/1",
        );
        assert_eq!(e[2]["elements"][0]["url"], "https://x/s/1");
    }

    #[test]
    fn the_home_tab_lists_what_needs_you() {
        let h = home(&Home {
            name: Some("Ada".into()),
            link: None,
            needs_you: vec![HomeSession {
                reference: "web#12".into(),
                title: "Fix it".into(),
                url: "https://x/s/1".into(),
                status: "Waiting for your answer".into(),
            }],
            running: Vec::new(),
            pulls: Vec::new(),
            app_url: "https://x".into(),
            can_start: true,
        });
        let text = h.to_string();
        assert!(text.contains("Needs you") && text.contains("No session of yours runs now."));
        assert!(text.contains(act::START));
        let unlinked = home(&Home {
            link: Some("https://x/api/v1/slack/link".into()),
            ..Home::default()
        });
        assert!(unlinked.to_string().contains("Link your Slack account"));
    }
}
