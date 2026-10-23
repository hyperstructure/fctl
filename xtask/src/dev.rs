//! `cargo xtask dev`: the whole system on one machine.
//!
//! - the control plane under `wrangler dev` (http://127.0.0.1:8787), with
//!   local D1 and Durable Objects;
//! - the web app under Vite with hot reload (http://localhost:5173);
//! - a host with the process driver whose agents run fake-claude, a scripted
//!   stand-in for Claude Code (no subscription, no network, deterministic).
//!
//! It builds the CLI, the host and the agent, the Worker and the web app at
//! once, bootstraps an owner, a Claude credential, a workspace, a runner and
//! a project the first time, and keeps its state in .fctl-dev/ (delete it to
//! start over).
//! With `--smoke` (CI), it runs one session to the end and stops.
//! With `--server <url>`, the host and the smoke test use a control plane
//! that already runs there instead (the standalone one, from `cargo xtask
//! serve` or its container image), which bootstraps with the dev setup token.

use crate::util::{self, run};
use anyhow::{Context, Result, bail};
use fctl_proto::api::{EventPage, MemoryList, RetrospectiveList};
use fctl_proto::event::{CommitInfo, EventBody, SessionEvent, StatusChange};
use fctl_proto::session::{Project, RetrospectiveOutcome, Session, SessionStatus};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const LOCAL_API: &str = "http://127.0.0.1:8787";
const WEB: &str = "http://localhost:5173";
pub const DEV_EMAIL: &str = "dev@fctl.test";
pub const DEV_PASSWORD: &str = "fctl-dev-password";
pub const SETUP_TOKEN: &str = "dev-setup-token";

static SERVER: OnceLock<String> = OnceLock::new();

/// The control plane's origin: wrangler dev's, or `--server`.
fn api() -> &'static str {
    SERVER.get().map_or(LOCAL_API, String::as_str)
}

pub struct DevOpts {
    /// Do not start a host (control plane and web app only).
    pub no_host: bool,
    /// Serve the built web app from wrangler instead of running Vite.
    pub no_vite: bool,
    /// Run one fake-claude session to the end, then stop everything (CI).
    pub smoke: bool,
    /// Use the control plane at this origin instead of starting wrangler dev.
    pub server: Option<String>,
}

struct Children(Vec<(&'static str, Child)>);

impl Children {
    /// Fails when one of them has exited.
    fn check(&mut self) -> Result<()> {
        for (name, c) in &mut self.0 {
            if let Some(status) = c.try_wait()? {
                bail!("{name} exited ({status})");
            }
        }
        Ok(())
    }
}

impl Drop for Children {
    /// Stops them as Ctrl-C does, then kills what still runs after 10 s.
    /// Killed straight away, wrangler's launcher would leave wrangler and
    /// workerd serving on 8787.
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            for (_, c) in &mut self.0 {
                if let Ok(None) = c.try_wait() {
                    let _ = Command::new("kill")
                        .args(["-INT", &c.id().to_string()])
                        .stderr(std::process::Stdio::null())
                        .status();
                }
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            for (_, c) in &mut self.0 {
                while Instant::now() < deadline && matches!(c.try_wait(), Ok(None)) {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
        for (_, c) in &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

pub fn dev(root: &Path, opts: DevOpts) -> Result<()> {
    if opts.smoke && opts.no_host {
        bail!("--smoke runs a session, which needs the host: leave out --no-host");
    }
    let mut state = root.join(".fctl-dev");
    let mut no_vite = opts.no_vite;
    if let Some(server) = &opts.server {
        // It serves its own web app, and isn't in development mode, so Vite's
        // origin can't sign in to it.
        no_vite = true;
        state = state.join("server");
        let _ = SERVER.set(server.trim_end_matches('/').to_owned());
    }
    std::fs::create_dir_all(&state)?;
    build(root, opts.server.is_none(), no_vite)?;
    let mut children = Children(Vec::new());
    if opts.server.is_none() {
        wrangler_dev(root, &mut children)?;
    }
    wait_for(
        &format!("{}/api/v1/health", api()),
        Duration::from_secs(300),
        &mut children,
    )?;

    let token = bootstrap(&state)?;
    if !no_vite {
        // Vite itself, not `npm run dev`: npm doesn't pass SIGINT on, and
        // stopping npm leaves Vite serving.
        let vite = util::without_package_env(
            Command::new(root.join("node_modules/.bin/vite"))
                .args(["--host", "localhost"])
                .current_dir(root.join("web")),
        )
        .spawn()
        .context("starting vite")?;
        children.0.push(("vite", vite));
    }
    if !opts.no_host {
        let host = host(root, &state, &token)?;
        children.0.push(("host", host));
    }

    if opts.smoke {
        if !no_vite {
            // The app, and the API through Vite's proxy.
            wait_for(
                &format!("{WEB}/api/v1/health"),
                Duration::from_secs(60),
                &mut children,
            )?;
        }
        eprintln!("\n\x1b[1mfctl dev is running\x1b[0m: starting one fake-claude session\n");
        return smoke(root, &token, &mut children);
    }

    let api = api();
    let open = if no_vite { api } else { WEB };
    let state_dir = if opts.server.is_some() {
        ".fctl-dev/server"
    } else {
        ".fctl-dev"
    };
    eprintln!(
        "\n\x1b[1mfctl dev is running\x1b[0m\n\n  \
         app       {open}\n  \
         sign in   {DEV_EMAIL} / {DEV_PASSWORD}\n  \
         api       {api}/api/v1\n  \
         cli       export FCTL_SERVER={api} FCTL_TOKEN=$(cat {state_dir}/token)\n\n\
         Agents run fake-claude: try a task like \"explore, then write notes.md\", \"rename Fix the login test\", \"screenshot\", \"remember Run npm ci first\", \"retro\", \"slow\", \"background 30\", \"delegate\", \"limit\" or \"crash\".\n\
         Ctrl-C stops everything. State lives in {state_dir}/.\n"
    );

    // Run until something exits (or Ctrl-C, which reaches every child too).
    loop {
        children.check()?;
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Builds the CLI, the host and the agent, and without `--server` what
/// `wrangler dev` runs, all at once: the Worker, the web app it serves, and
/// the local D1 with the migrations applied. The Worker builds in a target
/// directory of its own, so neither cargo waits for the other's lock.
fn build(root: &Path, local: bool, no_vite: bool) -> Result<()> {
    let bins = || {
        run(Command::new("cargo")
            .args([
                "build",
                "-p",
                "fctl",
                "-p",
                "fctl-agent",
                "-p",
                "fake-claude",
            ])
            .current_dir(root))
    };
    if !local {
        return bins();
    }
    std::thread::scope(|s| {
        let worker = util::beside(s, "worker", || build_worker(root));
        let web = util::beside(s, "web", || web_and_d1(root, no_vite));
        bins().and(util::join(worker)).and(util::join(web))
    })
}

/// The Worker, as `wrangler dev` builds it as it starts (`worker-build
/// --dev`, wrangler.toml), in the target directory it builds in: wrangler
/// then only has to bind it again.
fn build_worker(root: &Path) -> Result<()> {
    run(util::worker_build()?
        .arg("--dev")
        .env("CARGO_TARGET_DIR", worker_target(root))
        .current_dir(root.join("crates/fctl-control")))
}

/// Where `wrangler dev` builds the Worker, beside the host's builds.
fn worker_target(root: &Path) -> PathBuf {
    util::target_dir(root).join("worker")
}

/// The web app `wrangler dev` serves (Vite serves the live sources on top),
/// and the local D1 with the migrations applied.
fn web_and_d1(root: &Path, no_vite: bool) -> Result<()> {
    if !root.join("node_modules").exists() {
        util::npm_install(root)?;
    }
    if !root.join("web/dist/index.html").exists() || no_vite {
        util::build_web(root)?;
    }
    // CI=1: apply without asking (they are local and recorded in d1_migrations).
    run(util::wrangler(root)?
        .args(["d1", "migrations", "apply", "DB", "--local"])
        .env("CI", "1")
        .current_dir(root.join("crates/fctl-control")))
}

/// The control plane under `wrangler dev`, once `build` built what it runs.
fn wrangler_dev(root: &Path, children: &mut Children) -> Result<()> {
    dev_vars(root)?;
    let wrangler = util::without_package_env(
        util::wrangler(root)?
            .args([
                "dev",
                "--port",
                "8787",
                "--ip",
                "127.0.0.1",
                "--test-scheduled",
            ])
            .env("CARGO_TARGET_DIR", worker_target(root))
            .current_dir(root.join("crates/fctl-control")),
    )
    .spawn()
    .context("starting wrangler dev")?;
    children.0.push(("wrangler", wrangler));
    Ok(())
}

/// `--smoke`: starts a session with the CLI as a person would, on Haiku,
/// and fails unless the agent commits and pushes a file and the session
/// shows both while the turn still runs (the agent watches its checks
/// meanwhile), keeps a note in the project's memory, records a
/// retrospective and updates it (`fctl memory add`, `fctl retro`, `fctl
/// retro update`), the agent finishes its
/// turn, the session waits for the command and the subagent the agent left
/// working in the background and runs again while the agent takes each
/// one's result in a turn of its own, the
/// session stops when asked, it says which model and effort level Claude
/// worked at, and the project remembers the model it asked for.
fn smoke(root: &Path, token: &str, children: &mut Children) -> Result<()> {
    let fctl = || {
        let mut cmd = Command::new(util::target_dir(root).join("debug/fctl"));
        cmd.env("FCTL_SERVER", api()).env("FCTL_TOKEN", token);
        cmd
    };
    let out = util::output(fctl().args([
        "--json",
        "run",
        "--project",
        "playground",
        "--model",
        "haiku",
        "--detach",
        &format!(
            "write notes.md, push it, watch 12 seconds of checks, record a partial retro and revise it, then run the checks in the background 3 and delegate a review in the background\nremember {SMOKE_NOTE}"
        ),
    ]))?;
    let session: Session =
        serde_json::from_str(&out).context("reading the session `fctl run` started")?;
    let id = session.id.0;
    let res = finish(&id, token, children);
    if res.is_err() {
        // The transcript shows how far it got.
        let _ = run(fctl().args(["logs", &id]));
    }
    res
}

/// What the smoke test's agent keeps in the playground's memory.
const SMOKE_NOTE: &str = "The smoke test's agent writes notes.md and pushes it.";

fn finish(id: &str, token: &str, children: &mut Children) -> Result<()> {
    let auth = Auth::Bearer(token.to_owned());
    let mut seen = Seen::default();
    seen.until(
        id,
        &auth,
        children,
        Duration::from_secs(60),
        "show its commit and push, keep a note, record a retrospective and update it, finish its turn and wait for its background tasks",
        |s| match s.status() {
            Some(SessionStatus::Stopped) => bail!("the session stopped before its turn ended"),
            Some(SessionStatus::Idle) => Ok(s.turn_ok
                && s.commit.is_some()
                && s.waited_for_background
                && s.remembered
                && s.retrospective
                && s.retrospective_updated),
            _ => Ok(false),
        },
    )?;
    // The control plane keeps them for the people and the agents after it.
    let read = call("GET", "/projects/playground/memories", Some(&auth), None)?;
    ok("reading the project's memory", &read)?;
    let notes: MemoryList =
        serde_json::from_value(read.1).context("reading the project's memory")?;
    if !notes
        .memories
        .iter()
        .any(|m| m.text == SMOKE_NOTE && m.session.as_ref().is_some_and(|s| s.id.0 == id))
    {
        bail!("the project's memory has no note from the session that says {SMOKE_NOTE:?}");
    }
    let read = call(
        "GET",
        &format!("/sessions/{id}/retrospectives"),
        Some(&auth),
        None,
    )?;
    ok("reading the session's retrospectives", &read)?;
    let retros: RetrospectiveList =
        serde_json::from_value(read.1).context("reading the session's retrospectives")?;
    let [retro] = retros.retrospectives.as_slice() else {
        bail!(
            "the session has {} retrospectives, not the one its agent recorded and updated",
            retros.retrospectives.len()
        );
    };
    if retro.outcome != RetrospectiveOutcome::Completed || retro.updated_at.is_none() {
        bail!(
            "the session's retrospective says {:?}, updated at {:?}: its agent recorded it partial and updated it completed",
            retro.outcome,
            retro.updated_at
        );
    }
    // fake-claude says `running` from the turn that starts the subagent
    // until the turn that takes its report has ended, as Claude Code does,
    // so fctl-agent tells the turns of its own by their messages.
    seen.until(
        id,
        &auth,
        children,
        Duration::from_secs(60),
        "run while the agent takes its background tasks' results in turns of its own",
        |s| match &s.status {
            Some(StatusChange {
                status: SessionStatus::Stopped,
                ..
            }) => bail!("the session stopped while it waited for its background tasks"),
            Some(StatusChange {
                status: SessionStatus::Idle,
                reason,
                ..
            }) => Ok(s.results >= 3
                && s.ran_again >= 2
                && reason.as_deref() == Some("Waiting for a message")),
            _ => Ok(false),
        },
    )?;
    ok(
        "stopping the session",
        &call("POST", &format!("/sessions/{id}/stop"), Some(&auth), None)?,
    )?;
    seen.until(id, &auth, children, Duration::from_secs(60), "stop", |s| {
        Ok(s.status() == Some(SessionStatus::Stopped))
    })?;
    // The project sets no effort level, so fake-claude works at `medium`.
    let read = call("GET", &format!("/sessions/{id}"), Some(&auth), None)?;
    ok("reading the session", &read)?;
    let session: Session = serde_json::from_value(read.1).context("reading the session")?;
    if session.effort.as_deref() != Some("medium") {
        bail!(
            "the session says Claude worked at effort {:?}, not fake-claude's medium",
            session.effort
        );
    }
    // fake-claude says it runs the model it was given.
    if session.model.as_deref() != Some("haiku") {
        bail!(
            "the session says Claude worked with {:?}, not the haiku it asked for",
            session.model
        );
    }
    // The composer starts on it in the project next time.
    let read = call("GET", "/projects/playground", Some(&auth), None)?;
    ok("reading the project", &read)?;
    let project: Project = serde_json::from_value(read.1).context("reading the project")?;
    if project.last_model.as_deref() != Some("haiku") || project.last_effort.is_some() {
        bail!(
            "the project remembers model {:?} and effort {:?}, not haiku and none",
            project.last_model,
            project.last_effort
        );
    }
    let subject = seen.commit.map(|c| c.subject).unwrap_or_default();
    eprintln!(
        "\n\x1b[1msmoke test passed\x1b[0m: the agent committed \"{subject}\" and \
         pushed it, the session showed both before the turn ended, the agent \
         kept a note in the project's memory, recorded a retrospective and \
         updated it, \
         finished its turn, the session waited for its background command \
         and subagent and ran again while the agent took each one's result \
         in a turn of its own, the session stopped, \
         it said Claude worked with haiku at medium effort, and the project \
         remembers haiku\n"
    );
    Ok(())
}

/// What a session's events have shown so far.
#[derive(Default)]
struct Seen {
    after: u64,
    status: Option<StatusChange>,
    turn_ok: bool,
    results: u32,
    /// The session said it waited for background tasks.
    waited_for_background: bool,
    /// How often the session ran again after the first turn ended.
    ran_again: u32,
    commit: Option<CommitInfo>,
    /// The session showed the agent's commit pushed before the turn ended.
    pushed_in_turn: bool,
    /// The transcript showed a note the agent kept, a retrospective it
    /// recorded, and that retrospective updated.
    remembered: bool,
    retrospective: bool,
    retrospective_updated: bool,
}

impl Seen {
    fn status(&self) -> Option<SessionStatus> {
        self.status.as_ref().map(|s| s.status)
    }

    /// Reads new events until `done` says so. Fails when `done` does, when
    /// the session fails, when a child exits, or after `timeout`.
    fn until(
        &mut self,
        id: &str,
        auth: &Auth,
        children: &mut Children,
        timeout: Duration,
        what: &str,
        done: impl Fn(&Seen) -> Result<bool>,
    ) -> Result<()> {
        let start = Instant::now();
        loop {
            children.check()?;
            let path = format!("/sessions/{id}/events?after={}&limit=200", self.after);
            let res = call("GET", &path, Some(auth), None)?;
            ok("reading the session's events", &res)?;
            let page: EventPage =
                serde_json::from_value(res.1).context("reading the session's events")?;
            for e in &page.events {
                self.add(e)?;
            }
            if done(self)? {
                return Ok(());
            }
            if let Some(StatusChange {
                status: SessionStatus::Failed,
                reason,
                ..
            }) = &self.status
            {
                bail!("the session failed: {}", reason.as_deref().unwrap_or("?"));
            }
            if start.elapsed() > timeout {
                let status = self.status().map_or("none", |s| s.as_str());
                bail!(
                    "the session didn't {what} within {}s (status: {status})",
                    timeout.as_secs()
                );
            }
            if !page.has_more {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }

    fn add(&mut self, e: &SessionEvent) -> Result<()> {
        self.after = e.seq;
        match &e.body {
            EventBody::Status(s) => {
                if s.status == SessionStatus::Idle
                    && s.reason.as_deref() == Some("Waiting for 2 background tasks")
                {
                    self.waited_for_background = true;
                }
                if s.status == SessionStatus::Running && self.results > 0 {
                    self.ran_again += 1;
                }
                self.status = Some(s.clone());
            }
            EventBody::Claude(v) if e.body.claude_type() == Some("result") => {
                if v["is_error"].as_bool() == Some(true) {
                    bail!("the agent's turn failed: {v}");
                }
                if self.results == 0 && !self.pushed_in_turn {
                    bail!(
                        "the agent's first turn ended before the session showed its commit pushed"
                    );
                }
                self.turn_ok = true;
                self.results += 1;
            }
            EventBody::Git(g) if g.ahead > 0 => {
                self.commit = g.last_commit.clone();
                self.pushed_in_turn |= self.results == 0 && g.pushed;
            }
            EventBody::Notice(n) => {
                self.remembered |= n.code == "memory_added" && n.memory.is_some();
                self.retrospective |= n.code == "retrospective" && n.retrospective.is_some();
                self.retrospective_updated |=
                    n.code == "retrospective_updated" && n.retrospective.is_some();
            }
            _ => {}
        }
        Ok(())
    }
}

/// crates/fctl-control/.dev.vars with a master key and the dev setup token.
fn dev_vars(root: &Path) -> Result<()> {
    let path = root.join("crates/fctl-control/.dev.vars");
    if path.exists() {
        return Ok(());
    }
    std::fs::write(
        &path,
        format!(
            "# Written by cargo xtask dev.\nFCTL_MASTER_KEY={}\nFCTL_SETUP_TOKEN={SETUP_TOKEN}\n",
            util::random_key()?
        ),
    )?;
    Ok(())
}

fn wait_for(url: &str, timeout: Duration, children: &mut Children) -> Result<()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if ureq::get(url).call().is_ok() {
            return Ok(());
        }
        children.check()?;
        std::thread::sleep(Duration::from_millis(500));
    }
    bail!("{url} did not come up within {}s", timeout.as_secs())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into()
}

/// Calls the API; `auth` is either a bearer token or a cookie header.
fn call(
    method: &str,
    path: &str,
    auth: Option<&Auth>,
    body: Option<Value>,
) -> Result<(u16, Value, Option<String>)> {
    let url = format!("{}/api/v1{path}", api());
    let agent = agent();
    let mut req = ureq::http::Request::builder()
        .method(method)
        .uri(&url)
        .header("Origin", api())
        .header("Content-Type", "application/json");
    match auth {
        Some(Auth::Bearer(t)) => req = req.header("Authorization", format!("Bearer {t}")),
        Some(Auth::Cookie(c)) => req = req.header("Cookie", c.as_str()),
        None => {}
    }
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut res = agent.run(req.body(body)?)?;
    let status = res.status().as_u16();
    let cookie = res
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        // Without the `__Host-` prefix over plain http (fctl_domain::cookies).
        .find(|v| v.starts_with("__Host-fctl=") || v.starts_with("fctl="))
        .and_then(|v| v.split(';').next())
        .map(str::to_owned);
    let text = res.body_mut().read_to_string().unwrap_or_default();
    let json = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok((status, json, cookie))
}

enum Auth {
    Bearer(String),
    Cookie(String),
}

fn ok(what: &str, (status, body, _): &(u16, Value, Option<String>)) -> Result<()> {
    if !(200..300).contains(status) {
        bail!("{what}: HTTP {status}: {body}");
    }
    Ok(())
}

/// Makes sure the dev owner, a token, a Claude credential, a workspace and a
/// project exist.
/// Returns an API token for the owner.
fn bootstrap(state: &Path) -> Result<String> {
    let token_file = state.join("token");
    if let Ok(t) = std::fs::read_to_string(&token_file) {
        let t = t.trim().to_owned();
        if call("GET", "/auth/me", Some(&Auth::Bearer(t.clone())), None)?.0 == 200 {
            ensure_basics(&t)?;
            return Ok(t);
        }
    }
    let (_, instance, _) = call("GET", "/instance", None, None)?;
    let res = if instance["setup_required"].as_bool() == Some(true) {
        call(
            "POST",
            "/setup",
            None,
            Some(
                json!({ "token": SETUP_TOKEN, "email": DEV_EMAIL, "given_name": "Dev", "family_name": "Owner", "password": DEV_PASSWORD }),
            ),
        )?
    } else {
        call(
            "POST",
            "/auth/login",
            None,
            Some(json!({ "email": DEV_EMAIL, "password": DEV_PASSWORD })),
        )?
    };
    let what = if SERVER.get().is_some() {
        format!(
            "signing in as the dev owner (the control plane at {} makes it when it was started with FCTL_SETUP_TOKEN={SETUP_TOKEN} and has no account yet)",
            api()
        )
    } else {
        "signing in as the dev owner (delete .fctl-dev and crates/fctl-control/.wrangler to start over)".to_owned()
    };
    ok(&what, &res)?;
    if res.1.get("two_factor").is_some() {
        bail!(
            "the dev owner has two-factor sign-in, so a password alone can't sign it in: turn it off in Settings (or with `fctl account two-factor disable`), or delete .fctl-dev and crates/fctl-control/.wrangler to start over"
        );
    }
    let cookie = res.2.context("no session cookie in the sign-in response")?;
    let created = call(
        "POST",
        "/auth/tokens",
        Some(&Auth::Cookie(cookie)),
        Some(json!({ "name": "cargo xtask dev" })),
    )?;
    ok("creating an API token", &created)?;
    let token = created.1["secret"]
        .as_str()
        .context("no secret in the token response")?
        .to_owned();
    write_private(&token_file, &token)?;
    ensure_basics(&token)?;
    Ok(token)
}

fn ensure_basics(token: &str) -> Result<()> {
    let auth = Auth::Bearer(token.to_owned());
    let (_, me, _) = call("GET", "/auth/me", Some(&auth), None)?;
    if me["claude"].as_array().is_none_or(|a| a.is_empty()) {
        ok(
            "adding a Claude credential",
            &call(
                "POST",
                "/me/claude/token",
                Some(&auth),
                Some(
                    json!({ "token": "sk-ant-oat01-fctl-dev-fake-claude-0000000000", "label": "fake-claude" }),
                ),
            )?,
        )?;
    }
    let (_, workspaces, _) = call("GET", "/workspaces", Some(&auth), None)?;
    if workspaces["workspaces"]
        .as_array()
        .is_none_or(|a| a.is_empty())
    {
        ok(
            "creating the development workspace",
            &call(
                "POST",
                "/workspaces",
                Some(&auth),
                Some(json!({ "name": "Development", "color": "yellow" })),
            )?,
        )?;
    }
    let (_, projects, _) = call("GET", "/projects", Some(&auth), None)?;
    if projects["projects"].as_array().is_none_or(|a| a.is_empty()) {
        ok(
            "creating the playground project",
            &call(
                "POST",
                "/projects",
                Some(&auth),
                Some(
                    json!({ "name": "Playground", "description": "Sessions here run fake-claude." }),
                ),
            )?,
        )?;
    }
    Ok(())
}

/// Registers the dev runner once and starts `fctl host run` for it.
fn host(root: &Path, state: &Path, token: &str) -> Result<Child> {
    let config = state.join("host");
    std::fs::create_dir_all(&config)?;
    let token_file = config.join("host-token");
    if !token_file.exists() {
        let res = call(
            "POST",
            "/runners",
            Some(&Auth::Bearer(token.to_owned())),
            Some(json!({ "name": "dev-host" })),
        )?;
        ok("registering the dev runner", &res)?;
        let rt = res.1["token"].as_str().context("no runner token")?;
        write_private(&token_file, rt)?;
    }
    let bin = util::target_dir(root).join("debug");
    let toml = format!(
        "# Written by cargo xtask dev.\n\
         server = \"{api}\"\n\
         name = \"dev-host\"\n\
         driver = \"process\"\n\
         capacity = 4\n\
         claude_bin = {claude}\n\
         agent_bin = {agent}\n\
         data_dir = {data}\n\
         push_on_commit = false\n\
         # It runs what cargo builds, never a release.\n\
         auto_update = false\n\
         # Development machines and CI runners may have less free than the 20 GiB\n\
         # a host keeps by default, and would take no sessions.\n\
         min_free_disk_gb = 1\n",
        api = api(),
        claude = toml_str(&bin.join("fake-claude")),
        agent = toml_str(&bin.join("fctl-agent")),
        data = toml_str(&state.join("data")),
    );
    std::fs::write(config.join("host.toml"), toml)?;
    util::without_package_env(
        Command::new(bin.join("fctl"))
            .args(["host", "run"])
            .env("FCTL_CONFIG_DIR", &config)
            .env("FCTL_DATA_DIR", state.join("data"))
            .env(
                "RUST_LOG",
                std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
            ),
    )
    .spawn()
    .context("starting the dev host")
}

fn toml_str(p: &Path) -> String {
    toml::Value::String(p.display().to_string()).to_string()
}

fn write_private(path: &Path, value: &str) -> Result<()> {
    std::fs::write(path, format!("{value}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
