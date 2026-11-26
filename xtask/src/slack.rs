//! The Slack stand-in (xtask/slack) in `cargo xtask dev`: the control
//! plane's Slack app signs in to it and calls it as it would Slack, the dev
//! workspace works in its Slack workspace, Acme, the dev owner's account
//! there is linked, and #web-team's mentions start sessions in Playground.
//! `cargo xtask slack …` acts in Acme as people there do and follows what
//! fctl answers, and `--smoke` runs a mention through it.

use crate::dev::{self, Auth, Children, call, ok};
use crate::util;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// The stand-in: Slack's Web API and sign-in pages, and its own endpoints
/// under `/standin/`.
pub(crate) const STANDIN: &str = "http://127.0.0.1:8791";

/// fctl's Slack app in the dev stack, made up: the control plane signs in
/// to the stand-in with it, and the stand-in signs what it sends with the
/// signing secret.
const CLIENT_ID: &str = "1111111111.2222222222";
const CLIENT_SECRET: &str = "fctl-dev-slack-secret";
const SIGNING_SECRET: &str = "fctl-dev-slack-signing-secret";

/// Where the dev control plane says it is (`FCTL_PUBLIC_URL` in
/// wrangler.toml), in the addresses it sends browsers to.
const PUBLIC: &str = "http://localhost:8787";

/// The channel whose mentions start Playground's sessions in the dev stack.
const CHANNEL: &str = "web-team";

/// The control plane's variables that make the stand-in its Slack.
pub(crate) fn vars() -> Vec<(&'static str, String)> {
    vec![
        ("SLACK_CLIENT_ID", CLIENT_ID.to_owned()),
        ("SLACK_CLIENT_SECRET", CLIENT_SECRET.to_owned()),
        ("SLACK_SIGNING_SECRET", SIGNING_SECRET.to_owned()),
        ("SLACK_URL", STANDIN.to_owned()),
    ]
}

/// Starts the stand-in, with Acme and its log kept in `state`.
pub(crate) fn start(root: &Path, state: &Path, children: &mut Children) -> Result<()> {
    if !util::has("node") {
        bail!("node is not installed (Node.js 22 or newer runs the Slack stand-in)");
    }
    let base = format!("{}/api/v1/slack", dev::LOCAL_API);
    let standin = util::without_package_env(
        Command::new("node")
            .arg(root.join("xtask/slack/standin.mjs"))
            .args(["--port", "8791", "--state"])
            .arg(state)
            .arg("--events")
            .arg(format!("{base}/events"))
            .arg("--interactivity")
            .arg(format!("{base}/interactivity"))
            .arg("--commands")
            .arg(format!("{base}/commands"))
            .env("SLACK_CLIENT_ID", CLIENT_ID)
            .env("SLACK_CLIENT_SECRET", CLIENT_SECRET)
            .env("SLACK_SIGNING_SECRET", SIGNING_SECRET),
    )
    .spawn()
    .context("starting the Slack stand-in")?;
    children.0.push(("slack", standin));
    dev::wait_for(
        &format!("{STANDIN}/standin/health"),
        Duration::from_secs(30),
        children,
    )
}

/// Calls the stand-in's own endpoints: its answer, or what it said instead.
fn standin(method: &str, path: &str, body: Option<Value>) -> Result<Value> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();
    let req = ureq::http::Request::builder()
        .method(method)
        .uri(format!("{STANDIN}{path}"))
        .header("Content-Type", "application/json");
    let mut res = agent
        .run(req.body(body.map(|b| b.to_string()).unwrap_or_default())?)
        .with_context(|| {
            format!("the Slack stand-in at {STANDIN} didn't answer: start `cargo xtask dev` first")
        })?;
    let status = res.status().as_u16();
    let text = res.body_mut().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        bail!("{}", v["error"].as_str().unwrap_or(&text));
    }
    Ok(v)
}

/// Goes through a sign-in to Slack as a browser does, from `url` on: to
/// Slack's page, which sends it back to fctl, which keeps what came of it
/// and sends it on to a page of the web app, whose address says how it went
/// (`slack=connected`, `slack=linked`).
fn sign_in(url: &str, token: &str) -> Result<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();
    let mut url = url.replace(PUBLIC, dev::LOCAL_API);
    // The sign-in's cookie, which fctl sets as it sends the browser on.
    let mut jar: Vec<(String, String)> = Vec::new();
    for _ in 0..8 {
        // The stand-in's pages allow at once with `auto`, as a person
        // pressing Allow there would.
        if url.starts_with(STANDIN) && !url.contains("auto=") {
            url.push_str(if url.contains('?') {
                "&auto=1"
            } else {
                "?auto=1"
            });
        }
        let mut req = agent
            .get(&url)
            .header("Authorization", &format!("Bearer {token}"));
        if !jar.is_empty() {
            let cookies: Vec<String> = jar.iter().map(|(k, v)| format!("{k}={v}")).collect();
            req = req.header("Cookie", &cookies.join("; "));
        }
        let res = req.call()?;
        for set in res
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
        {
            if let Some((name, value)) = set.split(';').next().and_then(|p| p.split_once('=')) {
                jar.retain(|(n, _)| n != name.trim());
                if !value.is_empty() {
                    jar.push((name.trim().to_owned(), value.to_owned()));
                }
            }
        }
        let Some(location) = res.headers().get("location").and_then(|v| v.to_str().ok()) else {
            bail!(
                "{url} answered {} rather than send the browser on",
                res.status()
            );
        };
        let next = if location.starts_with("http") {
            location.to_owned()
        } else {
            let origin = url.split('/').take(3).collect::<Vec<_>>().join("/");
            format!("{origin}{location}")
        };
        let next = next.replace(PUBLIC, dev::LOCAL_API);
        if !next.contains("/api/v1/") && !next.starts_with(STANDIN) {
            return Ok(next);
        }
        url = next;
    }
    bail!("the sign-in to Slack went round in circles, from {url}")
}

/// Connects the workspace Playground is in to Acme, as an admin does under
/// Integrations → Slack, unless the stand-in has fctl's app installed for it
/// already; has #web-team's mentions start Playground's sessions and makes
/// #incidents readable; then links the dev owner's account in Acme, as
/// Settings does, unless it is linked.
pub(crate) fn connect(token: &str) -> Result<()> {
    let auth = Auth::Bearer(token.to_owned());
    let health = standin("GET", "/standin/health", None)?;
    let team = health["team"]["id"]
        .as_str()
        .context("the Slack stand-in names no team")?
        .to_owned();
    let project = call("GET", "/projects/playground", Some(&auth), None)?;
    ok("reading the playground project", &project)?;
    let project_id = project.1["id"]
        .as_str()
        .context("the playground project has no id")?
        .to_owned();
    let workspace = project.1["workspace"]["id"]
        .as_str()
        .context("the playground project is in no workspace")?
        .to_owned();
    let slack = call(
        "GET",
        &format!("/workspaces/{workspace}/slack"),
        Some(&auth),
        None,
    )?;
    ok("reading the dev workspace's Slack workspaces", &slack)?;
    let connection = slack.1["connections"]
        .as_array()
        .and_then(|c| c.iter().find(|c| c["team"] == team.as_str()).cloned());
    // The stand-in forgets fctl's app with .fctl-dev/slack, though the
    // control plane keeps the connection: then only the app is installed
    // again.
    if connection.as_ref().is_none_or(|c| c["status"] != "active") || health["installed"] != true {
        let res = call(
            "POST",
            &format!("/workspaces/{workspace}/slack/connect"),
            Some(&auth),
            Some(json!({})),
        )?;
        ok("connecting the dev workspace to the Slack stand-in", &res)?;
        let link = res.1["url"].as_str().context("no link in fctl's answer")?;
        let page = sign_in(link, token)?;
        if !page.contains("slack=connected") {
            bail!("connecting the dev workspace to the Slack stand-in ended at {page}");
        }
    }
    if connection.is_none() {
        let channel = |name: &str| -> Result<String> {
            let state = standin("GET", &format!("/standin/channels/{name}"), None)?;
            state["channel"]["id"]
                .as_str()
                .map(str::to_owned)
                .with_context(|| format!("the Slack stand-in has no #{name}"))
        };
        for (name, body) in [
            (CHANNEL, json!({ "name": CHANNEL, "project": project_id })),
            (
                "incidents",
                json!({ "name": "incidents", "readable": true }),
            ),
        ] {
            let id = channel(name)?;
            let res = call(
                "PUT",
                &format!("/workspaces/{workspace}/slack/{team}/channels/{id}"),
                Some(&auth),
                Some(body),
            )?;
            ok(&format!("choosing what #{name} does"), &res)?;
        }
    }
    let mine = call("GET", "/me/slack", Some(&auth), None)?;
    ok("reading the dev owner's Slack accounts", &mine)?;
    let linked = mine.1["accounts"]
        .as_array()
        .is_some_and(|a| a.iter().any(|a| a["team"] == team.as_str()));
    if !linked {
        let page = sign_in(
            &format!("{}/api/v1/slack/link?team={team}", dev::LOCAL_API),
            token,
        )?;
        if !page.contains("slack=linked") {
            bail!("linking the dev owner's account in the Slack stand-in ended at {page}");
        }
    }
    Ok(())
}

// ───────────────────────────── cargo xtask slack ─────────────────────────────

/// How long `cargo xtask slack` follows a thread for fctl's answer.
const FOLLOW: Duration = Duration::from_secs(180);

/// What someone does in Acme, as `/standin/<action>` takes it: what the
/// stand-in says they did, the thread it is in, and how fctl answered what
/// Slack sent.
fn act(action: &str, body: Value) -> Result<Value> {
    let done = standin("POST", &format!("/standin/{action}"), Some(body))?;
    let what = done["what"].as_str().unwrap_or(action).to_owned();
    if let Some(d) = done["delivery"].as_object() {
        let status = d.get("status").and_then(Value::as_u64).unwrap_or(0);
        if !(200..300).contains(&status) {
            bail!(
                "{what}, and fctl answered with {status}: {}",
                d.get("body").cloned().unwrap_or(Value::Null)
            );
        }
    }
    eprintln!("\x1b[1m{what}\x1b[0m");
    Ok(done)
}

/// `cargo xtask slack <action> [<target>] [<text>]`: someone in Acme (the
/// dev owner, or `--as` another) mentions fctl in a channel, writes to it,
/// replies in a thread, presses a button, runs `/fctl`, opens the Home tab
/// or presses stop, and this follows fctl's answer there; `show` shows a
/// thread, `uninstall` uninstalls fctl's app.
pub fn command(
    action: &str,
    target: Option<String>,
    text: Option<String>,
    who: Option<String>,
    channel: Option<String>,
) -> Result<()> {
    let channel = channel
        .map(|c| c.trim_start_matches('#').to_owned())
        .unwrap_or_else(|| CHANNEL.to_owned());
    let mut body = match action {
        // `mention "<text>" [--channel <name>]`.
        // `mention ['#<channel>'] "<text>"`, or `--channel <name>`.
        "mention" => {
            let (channel, text) = match (target, text) {
                (Some(c), t) if c.starts_with('#') => (c.trim_start_matches('#').to_owned(), t),
                (Some(a), Some(b)) => (channel, Some(format!("{a} {b}"))),
                (t, None) => (channel, t),
                (None, t) => (channel, t),
            };
            match text {
                Some(t) => json!({ "channel": channel, "text": t }),
                None => json!({ "channel": channel }),
            }
        }
        "dm" => json!({
            "text": target.context("say what you write to fctl: cargo xtask slack dm \"…\"")?,
        }),
        "reply" => match (target, text) {
            (Some(thread), Some(t)) => json!({ "thread": thread, "text": t }),
            (Some(t), None) => json!({ "text": t }),
            (None, _) => bail!(
                "say what the reply says: cargo xtask slack reply [<thread ts or link>] \"…\""
            ),
        },
        "answer" => json!({
            "answer": target
                .context("name the answer by its place: cargo xtask slack answer 0")?
                .parse::<u32>()
                .context("an answer is named by its place, from 0")?,
        }),
        "press" => json!({
            "label": target.context("name the button by its words: cargo xtask slack press \"Open in fctl\"")?,
        }),
        "project" => json!({
            "project": target.context("name the project: cargo xtask slack project playground")?,
            "remember": text.as_deref() == Some("--remember") || text.as_deref() == Some("remember"),
        }),
        "command" => {
            json!({ "text": target.unwrap_or_default(), "channel": text.unwrap_or_else(|| CHANNEL.to_owned()) })
        }
        "home" => json!({}),
        "stop" => match target {
            Some(t) => json!({ "thread": t }),
            None => json!({}),
        },
        "show" => {
            let path = match &target {
                Some(t) => format!("/standin/thread?ts={t}"),
                None => format!("/standin/channels/{channel}"),
            };
            show(&standin("GET", &path, None)?);
            return problems_note(0);
        }
        "uninstall" => {
            standin("POST", "/standin/uninstall", Some(json!({})))?;
            println!(
                "fctl's app was uninstalled from Acme: restart cargo xtask dev, or connect it again under Integrations → Slack"
            );
            return Ok(());
        }
        other => bail!(
            "unknown action `{other}`: mention, dm, reply, answer, press, project, command, home, stop, show or uninstall (cargo xtask help)"
        ),
    };
    if let Some(who) = who {
        body["as"] = json!(who);
    }
    // The problems from before are other actions'.
    let before = problems()?.len();
    let done = act(action, body)?;
    // Stop answers with nothing to follow.
    if let (Some(channel), Some(ts), false) = (
        done["thread"]["channel"]["id"].as_str(),
        done["thread"]["ts"].as_str(),
        action == "stop",
    ) {
        // What fctl posts after the person's message, or after now.
        let after = done["ts"].as_str().map(str::to_owned).unwrap_or_else(|| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            format!("{}.{:06}", now.as_secs(), now.subsec_micros())
        });
        follow(channel, ts, &after)?;
    }
    problems_note(before)
}

/// Prints what fctl posts in a thread from now on, until it replies or a
/// few minutes pass.
fn follow(channel: &str, ts: &str, after: &str) -> Result<()> {
    let path = format!("/standin/thread?channel={channel}&ts={ts}");
    let start = Instant::now();
    let mut seen = std::collections::HashSet::new();
    let mut replied = false;
    while start.elapsed() < FOLLOW && !replied {
        let t = standin("GET", &path, None)?;
        let messages = t["messages"].as_array().cloned().unwrap_or_default();
        for m in &messages {
            let Some(at) = m["ts"].as_str() else {
                continue;
            };
            if m["user"].as_str() != Some(BOT_USER)
                || ts_after(at, after).is_none_or(|later| !later)
                || !seen.insert(at.to_owned())
            {
                continue;
            }
            let words = message_words(m);
            println!("  fctl: {words}");
            replied |= !words.starts_with("card:");
        }
        // What fctl shows one person alone, in the thread or beside it in
        // the channel: a refusal, or the way to link their account, answers
        // them as a reply would.
        let beside = standin("GET", &format!("/standin/channels/{channel}"), None)?;
        let alone = t["ephemeral"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(beside["ephemeral"].as_array().into_iter().flatten());
        for m in alone {
            let Some(at) = m["ts"].as_str() else {
                continue;
            };
            if ts_after(at, after).is_none_or(|later| !later) || !seen.insert(format!("e{at}")) {
                continue;
            }
            let to = m["to"].as_str().unwrap_or("someone");
            let words = m["shown"]
                .as_str()
                .or(m["text"].as_str())
                .unwrap_or_default()
                .replace('\n', " · ");
            println!("  fctl, to {to} alone: {words}");
            replied = true;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    if !replied {
        println!("  (no reply from fctl yet: `cargo xtask slack show {ts}` shows the thread)");
    }
    Ok(())
}

/// Whether Slack's `ts` `a` comes after `b`, where both read as one.
fn ts_after(a: &str, b: &str) -> Option<bool> {
    let (a, b) = (a.parse::<f64>().ok()?, b.parse::<f64>().ok()?);
    Some(a > b)
}

/// fctl's bot user in the stand-in's Acme.
const BOT_USER: &str = "U0FCTLBOT";

/// What a message of fctl's says, in a line: its card's status, or its
/// words.
fn message_words(m: &Value) -> String {
    let blocks = m["blocks"].as_array().cloned().unwrap_or_default();
    let card = blocks
        .iter()
        .any(|b| b["elements"][0]["action_id"] == "fctl_open" && b["type"] == "actions")
        && blocks.len() >= 3;
    if card {
        let head = blocks[0]["text"]["text"].as_str().unwrap_or_default();
        return format!("card: {}", head.replace('\n', " · "));
    }
    let text = blocks
        .iter()
        .find_map(|b| b["text"].as_str().or_else(|| b["text"]["text"].as_str()))
        .or_else(|| m["text"].as_str())
        .unwrap_or_default();
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(160)
        .collect()
}

/// A thread or a channel, as the stand-in shows it.
fn show(state: &Value) {
    let messages = state["messages"].as_array().cloned().unwrap_or_default();
    if messages.is_empty() {
        println!("Nothing there yet: cargo xtask slack mention \"write notes.md\"");
    }
    for m in &messages {
        let who = if m["user"].as_str() == Some(BOT_USER) {
            "fctl".to_owned()
        } else {
            m["user"].as_str().unwrap_or("someone").to_owned()
        };
        println!(
            "{who} ({}): {}",
            m["ts"].as_str().unwrap_or_default(),
            message_words(m)
        );
    }
    if let Some(s) = state["agentSession"].as_object() {
        println!(
            "agent session: {}",
            s.get("status").and_then(Value::as_str).unwrap_or("none")
        );
    }
}

/// What the stand-in found that Slack would refuse, if anything.
fn problems_note(skip: usize) -> Result<()> {
    for problem in problems()?.iter().skip(skip) {
        eprintln!("\x1b[33mSlack would refuse:\x1b[0m {problem}");
    }
    Ok(())
}

/// What fctl sent that Slack would refuse, oldest first.
fn problems() -> Result<Vec<Value>> {
    let p = standin("GET", "/standin/problems", None)?;
    Ok(p["problems"].as_array().cloned().unwrap_or_default())
}

/// The smoke test's part in Slack: the dev owner mentions fctl in
/// #web-team, and fctl reacts, posts the session's card in the thread and,
/// once its agent finished its turn, its reply, with the session waiting in
/// Playground for a reply.
pub(crate) fn smoke(token: &str, children: &mut Children) -> Result<String> {
    let done = act(
        "mention",
        json!({ "channel": CHANNEL, "text": "write notes.md, then say what you did" }),
    )?;
    let ts = done["thread"]["ts"]
        .as_str()
        .context("the stand-in named no thread")?
        .to_owned();
    let path = format!("/standin/thread?channel={CHANNEL}&ts={ts}");
    let start = Instant::now();
    let (mut card, mut reply) = (false, false);
    while start.elapsed() < Duration::from_secs(240) && !(card && reply) {
        children.check()?;
        let t = standin("GET", &path, None)?;
        for m in t["messages"].as_array().into_iter().flatten() {
            if m["user"].as_str() != Some(BOT_USER) {
                continue;
            }
            if message_words(m).starts_with("card:") {
                card = true;
            } else {
                reply = true;
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    if !card {
        bail!("fctl posted no card in the thread of the mention in #{CHANNEL}");
    }
    if !reply {
        bail!("fctl's agent never replied in the thread of the mention in #{CHANNEL}");
    }
    // The session knows its thread.
    let auth = Auth::Bearer(token.to_owned());
    let sessions = call("GET", "/projects/playground/sessions", Some(&auth), None)?;
    ok("listing playground's sessions", &sessions)?;
    let from_slack = sessions.1["sessions"]
        .as_array()
        .is_some_and(|s| s.iter().any(|s| s["slack"]["thread_ts"] == ts.as_str()));
    if !from_slack {
        bail!("no session of Playground names the thread in #{CHANNEL} it started from");
    }
    // fctl's Home tab, which lists that session.
    let home = act("home", json!({ "wait": 10_000 }))?;
    if home["view"]["type"] != "home" {
        bail!("fctl published no Home tab for the dev owner as they opened it");
    }
    let p = standin("GET", "/standin/problems", None)?;
    if let Some(first) = p["problems"].as_array().and_then(|a| a.first()) {
        bail!("the Slack stand-in found what Slack would refuse: {first}");
    }
    Ok(format!(
        "a mention in #{CHANNEL} started a session, whose card and reply came to its thread, and the Home tab opened"
    ))
}
