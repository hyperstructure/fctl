//! The Linear stand-in (xtask/linear) in `cargo xtask dev`: the control
//! plane's Linear app signs in to it and asks it what it would ask Linear,
//! the dev workspace works in its Linear workspace, Acme, the dev owner's
//! account there is linked, and Playground takes the Engineering team's
//! tickets. `cargo xtask linear …` acts in Acme as people there do and
//! follows what fctl answers, and `--smoke` runs a delegation through it.

use crate::dev::{self, Auth, Children, call, ok};
use crate::util::{self, run};
use anyhow::{Context, Result, bail};
use fctl_proto::session::{Session, SessionStatus};
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// The stand-in: Linear's API and sign-in page, and its own endpoints
/// under `/standin/`.
pub(crate) const STANDIN: &str = "http://127.0.0.1:8790";

/// fctl's Linear app in the dev stack, made up: the control plane signs in
/// to the stand-in with it, and the stand-in signs its deliveries with the
/// webhook's secret.
const CLIENT_ID: &str = "fctl-dev-linear";
const CLIENT_SECRET: &str = "fctl-dev-linear-secret";
const WEBHOOK_SECRET: &str = "fctl-dev-linear-webhook-secret";

/// Where the dev control plane says it is (`FCTL_PUBLIC_URL` in
/// wrangler.toml), in the addresses it sends browsers to.
const PUBLIC: &str = "http://localhost:8787";

/// The control plane's variables that make the stand-in its Linear.
pub(crate) fn vars() -> Vec<(&'static str, String)> {
    vec![
        ("LINEAR_CLIENT_ID", CLIENT_ID.to_owned()),
        ("LINEAR_CLIENT_SECRET", CLIENT_SECRET.to_owned()),
        ("LINEAR_WEBHOOK_SECRET", WEBHOOK_SECRET.to_owned()),
        ("LINEAR_API_URL", STANDIN.to_owned()),
        ("LINEAR_AUTHORIZE_URL", format!("{STANDIN}/oauth/authorize")),
    ]
}

/// Starts the stand-in, with Acme and its log kept in `state`.
pub(crate) fn start(root: &Path, state: &Path, children: &mut Children) -> Result<()> {
    if !util::has("node") {
        bail!("node is not installed (Node.js 22 or newer runs the Linear stand-in)");
    }
    let standin = util::without_package_env(
        Command::new("node")
            .arg(root.join("xtask/linear/standin.mjs"))
            .args(["--port", "8790", "--state"])
            .arg(state)
            .arg("--webhook")
            .arg(format!("{}/api/v1/linear/webhook", dev::LOCAL_API))
            .env("LINEAR_CLIENT_ID", CLIENT_ID)
            .env("LINEAR_CLIENT_SECRET", CLIENT_SECRET)
            .env("LINEAR_WEBHOOK_SECRET", WEBHOOK_SECRET),
    )
    .spawn()
    .context("starting the Linear stand-in")?;
    children.0.push(("linear", standin));
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
            format!("the Linear stand-in at {STANDIN} didn't answer: start `cargo xtask dev` first")
        })?;
    let status = res.status().as_u16();
    let text = res.body_mut().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        bail!("{}", v["error"].as_str().unwrap_or(&text));
    }
    Ok(v)
}

/// What someone does in Acme (`delegate`, `mention`, `reply`, `stop`):
/// what the stand-in says they did, and the agent session once fctl took the
/// delivery.
fn act(action: &str, body: Value) -> Result<(String, Value)> {
    let done = standin("POST", &format!("/standin/{action}"), Some(body))?;
    let what = done["what"].as_str().unwrap_or(action).to_owned();
    let status = done["delivery"]["status"].as_u64().unwrap_or(0);
    if !(200..300).contains(&status) {
        bail!(
            "{what}, and fctl answered the delivery with {status}: {}",
            done["delivery"]["body"]
        );
    }
    Ok((what, done["session"].clone()))
}

/// Goes through a sign-in to Linear as a browser does, from `url` on: to
/// Linear's authorize page, which sends it back to fctl, which keeps what
/// came of it and sends it on to a page of the web app, whose address says
/// how it went (`linear=connected`, `linear=linked`).
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
    bail!("the sign-in to Linear went round in circles, from {url}")
}

/// The built CLI, signed in as the dev owner.
fn fctl(root: &Path, token: &str) -> Command {
    let mut cmd = Command::new(util::target_dir(root).join("debug/fctl"));
    cmd.env("FCTL_SERVER", dev::LOCAL_API)
        .env("FCTL_TOKEN", token);
    cmd
}

/// Connects the workspace Playground is in to Acme, as an admin does on
/// its Linear tab, and has Playground take Engineering's tickets, unless
/// the stand-in has fctl's app installed for it already; then links the
/// dev owner's account in Acme, as Settings does, unless it is linked.
pub(crate) fn connect(root: &Path, token: &str) -> Result<()> {
    let auth = Auth::Bearer(token.to_owned());
    let health = standin("GET", "/standin/health", None)?;
    let organization = health["organization"]["id"]
        .as_str()
        .context("the Linear stand-in names no organization")?
        .to_owned();
    let project = call("GET", "/projects/playground", Some(&auth), None)?;
    ok("reading the playground project", &project)?;
    let workspace = project.1["workspace"]["id"]
        .as_str()
        .context("the playground project is in no workspace")?
        .to_owned();
    let linear = call(
        "GET",
        &format!("/workspaces/{workspace}/linear"),
        Some(&auth),
        None,
    )?;
    ok("reading the dev workspace's Linear workspaces", &linear)?;
    let connected = linear.1["connections"]
        .as_array()
        .is_some_and(|c| c.iter().any(|c| c["organization"] == organization));
    // The stand-in forgets fctl's app with .fctl-dev/linear, though the
    // control plane keeps the connection: then only the app is installed
    // again, and Playground's tickets stay where someone may have moved them.
    if !connected || health["installed"] != true {
        let res = call(
            "POST",
            &format!("/workspaces/{workspace}/linear/connect"),
            Some(&auth),
            Some(json!({})),
        )?;
        ok("connecting the dev workspace to the Linear stand-in", &res)?;
        let link = res.1["url"].as_str().context("no link in fctl's answer")?;
        let page = sign_in(link, token)?;
        if !page.contains("linear=connected") {
            bail!("connecting the dev workspace to the Linear stand-in ended at {page}");
        }
    }
    if !connected {
        run(fctl(root, token).args([
            "projects",
            "set",
            "-p",
            "playground",
            "--tickets",
            "linear",
            "--linear-team",
            "ENG",
        ]))?;
    }
    let mine = call("GET", "/me/linear", Some(&auth), None)?;
    ok("reading the dev owner's Linear accounts", &mine)?;
    let linked = mine.1["accounts"]
        .as_array()
        .is_some_and(|a| a.iter().any(|a| a["organization"] == organization));
    if !linked {
        let page = sign_in(&format!("{}/api/v1/linear/link", dev::LOCAL_API), token)?;
        if !page.contains("linear=linked") {
            bail!("linking the dev owner's account in the Linear stand-in ended at {page}");
        }
    }
    Ok(())
}

// ───────────────────────────── cargo xtask linear ─────────────────────────────

/// How long `cargo xtask linear` follows an agent session for fctl's answer.
const FOLLOW: Duration = Duration::from_secs(180);

/// How long it waits for fctl to say anything at all: it answers each
/// delivery at once (a thought, or its answer).
const SILENCE: Duration = Duration::from_secs(15);

/// `cargo xtask linear <action> [<target>] [<text>]`: someone in Acme (the
/// dev owner, or `--as` another) delegates an issue to fctl, mentions it,
/// replies in an issue's agent session or presses stop, and this follows
/// fctl's answer there; `show` shows what fctl did in Acme, and `uninstall`
/// uninstalls fctl's app from it.
pub fn command(
    action: &str,
    target: Option<String>,
    text: Option<String>,
    who: Option<String>,
    description: Option<String>,
) -> Result<()> {
    let mut body = match action {
        "delegate" => issue(target, description)?,
        "mention" => {
            let mut body = issue(target, description)?;
            body["body"] = json!(text);
            body
        }
        "reply" => json!({
            "issue": target.context("name the issue: cargo xtask linear reply ENG-1 \"…\"")?,
            "body": text.context("say what the reply says: cargo xtask linear reply ENG-1 \"…\"")?,
        }),
        "stop" => {
            json!({ "issue": target.context("name the issue: cargo xtask linear stop ENG-1")? })
        }
        "show" => {
            let path = match &target {
                Some(t) => format!("/standin/sessions?issue={t}"),
                None => "/standin/sessions".to_owned(),
            };
            let all = standin("GET", &path, None)?;
            let sessions = all["sessions"].as_array().cloned().unwrap_or_default();
            if sessions.is_empty() {
                println!("fctl has no agent sessions there yet: cargo xtask linear delegate ENG-1");
            }
            for s in &sessions {
                show(s);
            }
            return problems_note();
        }
        "uninstall" => {
            standin("POST", "/standin/uninstall", Some(json!({})))?;
            println!(
                "fctl's app was uninstalled from Acme: restart cargo xtask dev, or connect it again on the workspace's Linear tab"
            );
            return Ok(());
        }
        other => bail!(
            "unknown action `{other}`: delegate, mention, reply, stop, show, uninstall or check (cargo xtask help)"
        ),
    };
    if let Some(who) = who {
        body["as"] = json!(who);
    }
    let (what, session) = act(action, body)?;
    follow(&what, &session)?;
    problems_note()
}

/// An issue by its key (`ENG-1`), or a new one with `target` for its title.
fn issue(target: Option<String>, description: Option<String>) -> Result<Value> {
    let t = target.context("name an issue (ENG-1), or give the title of a new one")?;
    let key = t.split_once('-').is_some_and(|(team, n)| {
        !team.is_empty()
            && team.chars().all(|c| c.is_ascii_alphabetic())
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
    });
    Ok(if key {
        json!({ "issue": t })
    } else {
        json!({ "title": t, "description": description })
    })
}

/// Prints what fctl posts in an agent session after someone's part in it
/// (`what`): all of a new one, whose first thought comes with fctl's answer
/// to the delivery. Until fctl waits for them (an answer, a question, an
/// error), [`FOLLOW`] passes, or fctl says nothing for [`SILENCE`].
fn follow(what: &str, session: &Value) -> Result<()> {
    let id = session["id"].as_str().context("no agent session")?;
    let seen = session["activities"]
        .as_array()
        .and_then(|a| a.iter().rposition(|a| a["app"] != true))
        .map_or(0, |i| i + 1);
    println!("{what}, in agent session {}:", &id[..8.min(id.len())]);
    let start = Instant::now();
    let mut printed = seen;
    loop {
        let s = standin("GET", &format!("/standin/sessions/{id}"), None)?;
        let activities = s["activities"].as_array().cloned().unwrap_or_default();
        for a in activities.iter().skip(printed) {
            if a["app"] == true {
                println!("  {}", activity(a));
            }
        }
        printed = activities.len();
        let waits = activities.iter().skip(seen).any(|a| {
            a["app"] == true
                && matches!(
                    a["type"].as_str(),
                    Some("response" | "elicitation" | "error")
                )
        });
        if waits {
            println!("{}", footer(&s));
            return Ok(());
        }
        if printed == seen && start.elapsed() > SILENCE {
            println!(
                "  … fctl said nothing in {} seconds: cargo xtask dev's output says why",
                SILENCE.as_secs()
            );
            return Ok(());
        }
        if start.elapsed() > FOLLOW {
            println!(
                "  … fctl is still at it: `cargo xtask linear show {}` shows what it did since",
                s["issue"].as_str().unwrap_or_default()
            );
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// An activity, in a line.
fn activity(a: &Value) -> String {
    let kind = a["type"].as_str().unwrap_or("?");
    let text = match kind {
        "action" => {
            let mut t = format!(
                "{} {}",
                a["action"].as_str().unwrap_or_default(),
                a["parameter"].as_str().unwrap_or_default()
            );
            if let Some(r) = a["result"].as_str() {
                t.push_str(&format!(" → {r}"));
            }
            t
        }
        _ => a["body"].as_str().unwrap_or_default().to_owned(),
    };
    let mut marks = Vec::new();
    if a["ephemeral"] == true {
        marks.push("ephemeral".to_owned());
    }
    if let Some(signal) = a["signal"].as_str() {
        match a["signalMetadata"]["url"].as_str() {
            Some(url) => marks.push(format!("{signal}: {url}")),
            None => marks.push(signal.to_owned()),
        }
    }
    let marks = if marks.is_empty() {
        String::new()
    } else {
        format!(" ({})", marks.join(", "))
    };
    format!(
        "{:<11} {}{marks}",
        kind,
        text.trim().replace('\n', "\n              ")
    )
}

/// Where an agent session's issue stands, and what it links to.
fn footer(s: &Value) -> String {
    let mut out = format!(
        "{} is {}",
        s["issue"].as_str().unwrap_or("?"),
        s["state"].as_str().unwrap_or("?")
    );
    for l in s["externalUrls"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "; {}: {}",
            l["label"].as_str().unwrap_or("link"),
            l["url"].as_str().unwrap_or_default()
        ));
    }
    out
}

/// An agent session, as `show` prints it.
fn show(s: &Value) {
    println!(
        "{} {} · agent session {} by {}, {}",
        s["issue"].as_str().unwrap_or("?"),
        s["title"].as_str().unwrap_or_default(),
        s["id"].as_str().map_or("?", |id| &id[..8.min(id.len())]),
        s["creator"].as_str().unwrap_or("someone"),
        s["status"].as_str().unwrap_or("?")
    );
    if let Some(m) = s["mention"].as_str() {
        println!("  {:<11} {m}", "mention");
    }
    for a in s["activities"].as_array().into_iter().flatten() {
        if a["app"] == true {
            println!("  {}", activity(a));
        } else {
            let text = match a["signal"].as_str() {
                Some(signal) => format!("({signal})"),
                None => a["body"].as_str().unwrap_or_default().to_owned(),
            };
            println!(
                "  {:<11} {}: {text}",
                "prompt",
                a["by"].as_str().unwrap_or("someone")
            );
        }
    }
    println!("  {}\n", footer(s));
}

/// Says what Linear would have refused of what fctl sent, if anything.
fn problems_note() -> Result<()> {
    let p = standin("GET", "/standin/problems", None)?;
    let list = p["problems"].as_array().cloned().unwrap_or_default();
    if !list.is_empty() {
        eprintln!(
            "\nThe stand-in found {} problem(s) since it started; the first: {}",
            list.len(),
            list[0]["message"].as_str().unwrap_or("?")
        );
    }
    Ok(())
}

// ───────────────────────────── cargo xtask linear check ─────────────────────────────

/// Checks every query and mutation fctl sends Linear
/// (`fctl_domain::linear::operations`), with variables of the kinds the
/// control plane sends, against Linear's schema as the stand-in takes them
/// (xtask/linear/check.mjs). Nothing runs, so it needs no stack.
pub(crate) fn check(root: &Path) -> Result<()> {
    if !util::npm_installed(root) {
        util::npm_install(root)?;
    }
    let mut operations = Vec::new();
    for (name, query) in fctl_domain::linear::operations() {
        let forms = samples(name).with_context(|| {
            format!(
                "xtask/src/linear.rs has no variables to check {name} with: add them to `samples`"
            )
        })?;
        for variables in forms {
            operations.push(json!({ "name": name, "query": query, "variables": variables }));
        }
    }
    util::run_with_stdin(
        Command::new("node").arg(root.join("xtask/linear/check.mjs")),
        &json!({ "operations": operations }).to_string(),
    )
}

/// Variables of the kinds the control plane sends with each of fctl's
/// operations (`fctl_domain::linear::operations`), by its name: a set for
/// each form it takes, made with fctl's own functions where it has them.
/// What the smoke test's delegation sends is checked as it is sent.
fn samples(name: &str) -> Option<Vec<Value>> {
    use fctl_domain::linear::{self as lin, Act};
    // Linear's ids are UUIDs; the check looks none of them up.
    const TEAM: &str = "0ab655aa-2e07-486f-ac77-e8c0ae7a5af4";
    const PROJECT: &str = "79a8a551-4658-4e5b-9bda-03b655e25b2a";
    const ISSUE: &str = "2f48112c-0dff-4a10-b0f1-feb30ba83f6e";
    const SESSION: &str = "144dd6dd-5480-43f8-8f3d-78207d73753f";
    const LABEL: &str = "96122a4c-6dac-4d45-bd3c-92f1fb4ffd8d";
    const STATE: &str = "48563dda-2696-48f5-9085-e6f7eaf2d0b5";
    const COMMENT: &str = "f10a2801-9268-4155-b704-d06f75aa2471";
    const USER: &str = "0d842641-32ca-405f-b4a1-48e5629f8ef5";
    let filter = json!({
        "team": { "id": { "eq": TEAM } },
        "project": { "id": { "eq": PROJECT } },
        "state": { "type": { "in": ["triage", "backlog", "unstarted", "started"] } },
        "labels": { "name": { "eqIgnoreCase": "bug" } },
    });
    let pull = "https://github.com/acme/web/pull/12";
    Some(match name {
        "VIEWER" | "TEAM_IDS" => vec![json!({})],
        "TEAMS" => vec![
            json!({ "first": lin::TEAMS_PAGE, "after": null }),
            json!({ "first": 20, "after": "cursor" }),
        ],
        "TEAM" | "TEAM_STATES" | "TEAM_LABELS" => vec![json!({ "id": TEAM })],
        "PROJECT" => vec![json!({ "id": PROJECT })],
        "issue_query" => vec![json!({ "id": "ENG-123" }), json!({ "id": ISSUE })],
        "ISSUE_STATES" => vec![json!({ "id": ISSUE })],
        "issues_query" => vec![json!({ "filter": filter, "first": 50 })],
        "search_query" => vec![json!({ "term": "footer links", "filter": filter, "first": 50 })],
        "issue_create" => vec![json!({ "input": {
            "teamId": TEAM, "title": "Fix the footer's links", "description": "Two links lead nowhere.",
            "projectId": PROJECT, "parentId": ISSUE, "labelIds": [LABEL], "stateId": STATE,
            "priority": lin::priority("high").ok()?,
        } })],
        "issue_update" => vec![
            json!({ "id": ISSUE, "input": {
                "title": "Fix the footer's links", "description": "Two links lead nowhere.", "stateId": STATE,
                "addedLabelIds": [LABEL], "removedLabelIds": [LABEL], "parentId": ISSUE,
                "priority": lin::priority("none").ok()?,
            } }),
            json!({ "id": ISSUE, "input": { "parentId": null } }),
        ],
        "COMMENT_CREATE" => vec![
            json!({ "input": { "issueId": ISSUE, "body": "Looked into it." } }),
            json!({ "input": { "issueId": ISSUE, "body": "Done.", "parentId": COMMENT } }),
        ],
        "ACTIVITY_CREATE" => {
            let reading = Act::Action {
                action: "Reading",
                parameter: "src/main.rs",
                result: None,
            };
            let opened = Act::Action {
                action: "Opened a pull request",
                parameter: "acme/web#12",
                result: Some(pull),
            };
            let projects = [
                ("Web".to_owned(), "web".to_owned()),
                ("API".to_owned(), "api".to_owned()),
            ];
            [
                lin::activity(SESSION, &Act::Thought("Looking at ENG-123…"), true),
                lin::activity(SESSION, &reading, true),
                lin::activity(SESSION, &opened, false),
                lin::activity(SESSION, &Act::Elicitation("Which page?"), false),
                lin::activity(SESSION, &Act::Response("Done."), false),
                lin::activity(
                    SESSION,
                    &Act::Error(&lin::couldnt_start("ENG-123", None)),
                    false,
                ),
                lin::link_account(
                    SESSION,
                    "Connect your Linear account to fctl once.",
                    "http://localhost:8787/api/v1/linear/link?thread=x",
                    Some(USER),
                ),
                lin::choose_project(SESSION, "Which project should work on it?", &projects),
            ]
            .into_iter()
            .map(|input| json!({ "input": input }))
            .collect()
        }
        "SESSION_UPDATE" => {
            let todos = json!([
                { "content": "Read the code", "status": "completed" },
                { "content": "Fix the links", "status": "in_progress" },
                { "content": "Test them", "status": "pending" },
            ]);
            vec![
                json!({ "id": SESSION, "input": { "externalUrls": [
                    { "label": "fctl session", "url": "http://localhost:8787/s/ses_1" },
                    { "label": "acme/web#12", "url": pull },
                ] } }),
                json!({ "id": SESSION, "input": { "plan": lin::plan(&todos)? } }),
            ]
        }
        "ATTACHMENT_LINK_URL" => {
            vec![json!({ "issueId": ISSUE, "url": pull, "title": "Fix the footer's links" })]
        }
        "REPOSITORY_SUGGESTIONS" => vec![json!({
            "issueId": ISSUE, "agentSessionId": SESSION,
            "candidates": [
                { "hostname": "github.com", "repositoryFullName": "acme/web" },
                { "hostname": "github.com", "repositoryFullName": "acme/api" },
            ],
        })],
        "CREATOR_TIMEZONE" => vec![json!({ "id": SESSION })],
        _ => return None,
    })
}

// ───────────────────────────── --smoke ─────────────────────────────

/// What fctl posted in an agent session, of `kind`, whose text starts with `start`.
fn posted<'a>(s: &'a Value, kind: &str, start: &str) -> Option<&'a Value> {
    s["activities"].as_array()?.iter().find(|a| {
        a["app"] == true
            && a["type"] == kind
            && a["body"].as_str().is_some_and(|b| b.starts_with(start))
    })
}

/// How many activities an agent session has: what came after is new.
fn count(s: &Value) -> usize {
    s["activities"].as_array().map_or(0, Vec::len)
}

/// Whether fctl posted an activity of `kind` in an agent session after its first `n`.
fn posted_since(s: &Value, n: usize, kind: &str) -> bool {
    s["activities"].as_array().is_some_and(|a| {
        a.iter()
            .skip(n)
            .any(|a| a["app"] == true && a["type"] == kind)
    })
}

/// Reads an agent session from the stand-in until `done` says so. Fails
/// when a child exits, or after `timeout`, with what fctl posted there.
fn until(
    id: &str,
    children: &mut Children,
    timeout: Duration,
    what: &str,
    done: impl Fn(&Value) -> bool,
) -> Result<Value> {
    let start = Instant::now();
    loop {
        children.check()?;
        let s = standin("GET", &format!("/standin/sessions/{id}"), None)?;
        if done(&s) {
            return Ok(s);
        }
        if start.elapsed() > timeout {
            let lines: Vec<String> = s["activities"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|a| format!("  {}: {}", a["by"].as_str().unwrap_or("?"), activity(a)))
                .collect();
            bail!(
                "fctl didn't {what} within {}s; the agent session in Linear shows:\n{}",
                timeout.as_secs(),
                lines.join("\n")
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Reads fctl session `id` until its status is one `want` takes.
fn status_until(
    auth: &Auth,
    id: &str,
    children: &mut Children,
    timeout: Duration,
    what: &str,
    want: impl Fn(SessionStatus) -> bool,
) -> Result<()> {
    let start = Instant::now();
    loop {
        children.check()?;
        let res = call("GET", &format!("/sessions/{id}"), Some(auth), None)?;
        ok("reading the session", &res)?;
        let s: Session = serde_json::from_value(res.1).context("reading the session")?;
        if want(s.status) {
            return Ok(());
        }
        if s.status == SessionStatus::Failed {
            bail!(
                "the session the delegation started failed: {}",
                s.status_reason.as_deref().unwrap_or("?")
            );
        }
        if start.elapsed() > timeout {
            bail!(
                "the session the delegation started didn't {what} within {}s (status: {})",
                timeout.as_secs(),
                s.status.as_str()
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// The fctl session an agent session links to.
fn linked_session(s: &Value) -> Option<String> {
    s["externalUrls"]
        .as_array()?
        .iter()
        .find(|l| l["label"] == "fctl session")?["url"]
        .as_str()?
        .rsplit_once("/s/")
        .map(|(_, id)| id.to_owned())
}

/// `--smoke`'s delegation, in the stand-in: someone delegates a new issue
/// to fctl, which starts a session of Playground, says so in the issue's
/// agent session, moves the issue to In Progress, links the session there
/// and answers as the turn ends; their reply has the agent list, read,
/// file, comment on and change tickets with `fctl ticket`, which reach
/// Acme; another reply reaches the agent and their stop interrupts its
/// turn; another runs into a usage limit, and fctl says until when the
/// session pauses, in their time zone; a reply from someone fctl doesn't
/// know asks them to link their account; a mention of fctl in another issue
/// starts a session that answers there; an issue of a team no project takes
/// gets fctl's refusal; and the stand-in found nothing Linear would have
/// refused, or fctl didn't do in time. Returns what it saw, for the smoke
/// test's summary.
pub(crate) fn smoke(root: &Path, token: &str, children: &mut Children) -> Result<String> {
    let auth = Auth::Bearer(token.to_owned());
    let minute = Duration::from_secs(60);
    // Playground takes Engineering's tickets, as `connect` left it, unless
    // someone chose otherwise since.
    let project = call("GET", "/projects/playground", Some(&auth), None)?;
    ok("reading the playground project", &project)?;
    if project.1["settings"]["tickets"]["tracker"] != "linear" {
        run(fctl(root, token).args([
            "projects",
            "set",
            "-p",
            "playground",
            "--tickets",
            "linear",
            "--linear-team",
            "ENG",
        ]))?;
    }

    let (_, delegated) = act(
        "delegate",
        json!({ "title": "Say what the playground holds", "description": "Say what the repository holds." }),
    )?;
    let id = delegated["id"].as_str().context("no agent session")?;
    let key = delegated["issue"].as_str().unwrap_or("?").to_owned();
    let s = until(
        id,
        children,
        minute,
        &format!("answer the delegation of {key}"),
        |s| posted(s, "response", "").is_some(),
    )?;
    if posted(&s, "thought", &format!("Looking at {key}")).is_none()
        || posted(&s, "thought", "Working on this in fctl session playground#").is_none()
    {
        bail!(
            "fctl answered the delegation of {key} without saying it looks at it and works on it in a session: {s}"
        );
    }
    if s["state"] != "In Progress" {
        bail!(
            "fctl left {key} in {} rather than move it to In Progress",
            s["state"]
        );
    }
    let session = linked_session(&s).with_context(|| {
        format!(
            "the agent session on {key} links no fctl session: {}",
            s["externalUrls"]
        )
    })?;

    // The agent works with the project's tickets, as agents reach their
    // project's tracker: fake-claude runs the lines that start `fctl ticket`.
    let before = count(&s);
    let tickets = format!(
        "fctl ticket\n\
         fctl ticket list --label feature\n\
         fctl ticket list --all --search playground\n\
         fctl ticket view ENG-1\n\
         fctl ticket create --title \"Say how to run the tests\" --parent {key} --label Feature --state Backlog --priority low\n\
         fctl ticket comment {key} Looked at what the repository holds.\n\
         fctl ticket update {key} --state \"In Review\" --add-label Bug --priority high\n\
         fctl ticket states\n\
         fctl ticket labels"
    );
    act("reply", json!({ "session": id, "body": tickets }))?;
    until(
        id,
        children,
        minute,
        "answer the reply that works with tickets",
        |s| posted_since(s, before, "response"),
    )?;
    let issue = standin("GET", &format!("/standin/issues/{key}"), None)?;
    let commented = issue["comments"].as_array().is_some_and(|c| {
        c.iter()
            .any(|c| c["by"] == "fctl" && c["body"] == "Looked at what the repository holds.")
    });
    let labels = issue["labels"].as_array().cloned().unwrap_or_default();
    if !commented
        || issue["state"] != "In Review"
        || issue["priority"] != "High"
        || !labels.contains(&json!("Bug"))
    {
        bail!(
            "the agent's `fctl ticket comment` and `update` didn't reach {key} in Linear: {issue}"
        );
    }
    let child = issue["children"]
        .as_array()
        .and_then(|c| c.last())
        .and_then(Value::as_str)
        .with_context(|| format!("the agent's `fctl ticket create` filed no sub-issue of {key}"))?;
    let filed = standin("GET", &format!("/standin/issues/{child}"), None)?;
    if filed["title"] != "Say how to run the tests"
        || filed["state"] != "Backlog"
        || filed["priority"] != "Low"
        || filed["labels"] != json!(["Feature"])
        || filed["creator"] != "fctl"
    {
        bail!("the agent's `fctl ticket create` filed {child} otherwise than asked: {filed}");
    }

    // A reply that starts a long turn, and a stop that ends it.
    act(
        "reply",
        json!({ "session": id, "body": "watch 30 seconds of checks" }),
    )?;
    status_until(
        &auth,
        &session,
        children,
        minute,
        "run the turn of the reply",
        |st| st == SessionStatus::Running,
    )?;
    act("stop", json!({ "session": id }))?;
    until(id, children, minute, "say it stopped", |s| {
        posted(s, "response", "Stopped. Reply here to carry on.").is_some()
    })?;
    status_until(
        &auth,
        &session,
        children,
        Duration::from_secs(20),
        "end the turn its stop interrupted",
        |st| st == SessionStatus::Idle,
    )?;

    // A usage limit: the session pauses, and says until when, in the zone of
    // the person who delegated the issue (Dev Owner's, in Acme).
    act("reply", json!({ "session": id, "body": "limit" }))?;
    let s = until(id, children, minute, "say the session pauses", |s| {
        posted(s, "thought", "The session pauses until").is_some()
    })?;
    let note = posted(&s, "thought", "The session pauses until")
        .map_or("", |a| a["body"].as_str().unwrap_or_default());
    if !note.contains("(Europe/Stockholm)") {
        bail!(
            "fctl's pause note in Linear isn't in Dev Owner's time zone, Europe/Stockholm: {note}"
        );
    }

    // Someone fctl doesn't know replies: they're asked to link their account.
    act(
        "reply",
        json!({ "session": id, "body": "Can I help?", "as": "grace" }),
    )?;
    until(id, children, minute, "ask Grace to link her account", |s| {
        s["activities"].as_array().is_some_and(|a| {
            a.iter()
                .any(|a| a["app"] == true && a["type"] == "elicitation" && a["signal"] == "auth")
        })
    })?;

    // A mention in another issue starts another session, which answers there.
    let (_, mentioned) = act(
        "mention",
        json!({ "title": "Count what the playground holds", "body": "how many files does the playground hold?" }),
    )?;
    let mid = mentioned["id"].as_str().context("no agent session")?;
    let m = until(mid, children, minute, "answer the mention", |s| {
        posted(s, "response", "").is_some()
    })?;
    let other = linked_session(&m).context("the mention's agent session links no fctl session")?;

    // An issue of a team no project takes: fctl says so, and starts nothing.
    let (_, refused) = act(
        "delegate",
        json!({ "title": "Draw the error states", "team": "DES" }),
    )?;
    let rid = refused["id"].as_str().context("no agent session")?;
    until(
        rid,
        children,
        minute,
        "say no project takes DES's tickets",
        |s| posted(s, "error", "No fctl project takes DES's tickets yet").is_some(),
    )?;

    let problems = standin("GET", "/standin/problems", None)?;
    if let Some(list) = problems["problems"].as_array().filter(|l| !l.is_empty()) {
        let lines: Vec<String> = list
            .iter()
            .map(|p| format!("  {}", p["message"].as_str().unwrap_or("?")))
            .collect();
        bail!(
            "the Linear stand-in found what Linear would have refused, or fctl didn't do in time:\n{}",
            lines.join("\n")
        );
    }
    for sid in [&session, &other] {
        ok(
            "stopping a session the Linear stand-in started",
            &call("POST", &format!("/sessions/{sid}/stop"), Some(&auth), None)?,
        )?;
    }
    Ok(format!(
        "in the Linear stand-in, a delegation of {key} started a session that said so there, moved the issue to In Progress, linked the session and answered as its turn ended, the agent's `fctl ticket` commands listed, read, filed, commented on and changed issues there, a reply reached the agent and a stop interrupted it, a usage limit's pause note came in Dev Owner's time zone, a reply from someone fctl doesn't know asked them to link their account, a mention started another session that answered, a delegation no project takes got fctl's refusal, and Linear's schema took everything fctl sent"
    ))
}
