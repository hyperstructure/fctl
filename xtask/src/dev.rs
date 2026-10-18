//! `cargo xtask dev`: the whole system on one machine.
//!
//! - the control plane under `wrangler dev` (http://127.0.0.1:8787), with
//!   local D1 and Durable Objects;
//! - the web app under Vite with hot reload (http://localhost:5173);
//! - a host with the process driver whose agents run fake-claude, a scripted
//!   stand-in for Claude Code (no subscription, no network, deterministic).
//!
//! It bootstraps an owner, a Claude credential, a runner and a project the
//! first time, and keeps its state in .fctl-dev/ (delete it to start over).
//! With `--smoke` (CI), it runs one session to the end and stops.

use crate::util::{self, run};
use anyhow::{Context, Result, bail};
use fctl_proto::api::EventPage;
use fctl_proto::event::{CommitInfo, EventBody, SessionEvent, StatusChange};
use fctl_proto::session::{Session, SessionStatus};
use serde_json::{Value, json};
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

const API: &str = "http://127.0.0.1:8787";
const WEB: &str = "http://localhost:5173";
pub const DEV_EMAIL: &str = "dev@fctl.test";
pub const DEV_PASSWORD: &str = "fctl-dev-password";
const SETUP_TOKEN: &str = "dev-setup-token";

pub struct DevOpts {
    /// Do not start a host (control plane and web app only).
    pub no_host: bool,
    /// Serve the built web app from wrangler instead of running Vite.
    pub no_vite: bool,
    /// Run one fake-claude session to the end, then stop everything (CI).
    pub smoke: bool,
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
    let state = root.join(".fctl-dev");
    std::fs::create_dir_all(&state)?;
    if !root.join("node_modules").exists() {
        util::npm_install(root)?;
    }
    // wrangler dev builds the Worker with `worker-build --dev` (wrangler.toml):
    // this installs it when it is missing.
    util::worker_build()?;
    dev_vars(root)?;
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
        .current_dir(root))?;
    // wrangler serves web/dist; Vite serves the live sources on top.
    if !root.join("web/dist/index.html").exists() || opts.no_vite {
        util::build_web(root)?;
    }

    // CI=1: apply without asking (they are local and recorded in d1_migrations).
    run(util::wrangler(root)?
        .args(["d1", "migrations", "apply", "DB", "--local"])
        .env("CI", "1")
        .current_dir(root.join("crates/fctl-control")))?;

    let mut children = Children(Vec::new());
    let wrangler = util::wrangler(root)?
        .args([
            "dev",
            "--port",
            "8787",
            "--ip",
            "127.0.0.1",
            "--test-scheduled",
        ])
        .current_dir(root.join("crates/fctl-control"))
        .spawn()
        .context("starting wrangler dev")?;
    children.0.push(("wrangler", wrangler));
    wait_for(
        &format!("{API}/api/v1/health"),
        Duration::from_secs(300),
        &mut children,
    )?;

    let token = bootstrap(&state)?;
    if !opts.no_vite {
        // Vite itself, not `npm run dev`: npm doesn't pass SIGINT on, and
        // stopping npm leaves Vite serving.
        let vite = Command::new(root.join("node_modules/.bin/vite"))
            .args(["--host", "localhost"])
            .current_dir(root.join("web"))
            .spawn()
            .context("starting vite")?;
        children.0.push(("vite", vite));
    }
    if !opts.no_host {
        let host = host(root, &state, &token)?;
        children.0.push(("host", host));
    }

    if opts.smoke {
        if !opts.no_vite {
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

    let open = if opts.no_vite { API } else { WEB };
    eprintln!(
        "\n\x1b[1mfctl dev is running\x1b[0m\n\n  \
         app       {open}\n  \
         sign in   {DEV_EMAIL} / {DEV_PASSWORD}\n  \
         api       {API}/api/v1\n  \
         cli       export FCTL_SERVER={API} FCTL_TOKEN=$(cat .fctl-dev/token)\n\n\
         Agents run fake-claude: try a task like \"write notes.md\", \"rename Fix the login test\", \"screenshot\", \"slow\", \"background 30\", \"limit\" or \"crash\".\n\
         Ctrl-C stops everything. State lives in .fctl-dev/.\n"
    );

    // Run until something exits (or Ctrl-C, which reaches every child too).
    loop {
        children.check()?;
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// `--smoke`: starts a session with the CLI as a person would, and fails
/// unless the agent commits a file and finishes its turn, and the session
/// stops when asked.
fn smoke(root: &Path, token: &str, children: &mut Children) -> Result<()> {
    let fctl = || {
        let mut cmd = Command::new(util::target_dir(root).join("debug/fctl"));
        cmd.env("FCTL_SERVER", API).env("FCTL_TOKEN", token);
        cmd
    };
    let out = util::output(fctl().args([
        "--json",
        "run",
        "--project",
        "playground",
        "--detach",
        "write notes.md",
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

fn finish(id: &str, token: &str, children: &mut Children) -> Result<()> {
    let auth = Auth::Bearer(token.to_owned());
    let mut seen = Seen::default();
    seen.until(
        id,
        &auth,
        children,
        Duration::from_secs(60),
        "finish its turn with a commit",
        |s| match s.status() {
            Some(SessionStatus::Stopped) => bail!("the session stopped before its turn ended"),
            Some(SessionStatus::Idle) => Ok(s.turn_ok && s.commit.is_some()),
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
    let subject = seen.commit.map(|c| c.subject).unwrap_or_default();
    eprintln!(
        "\n\x1b[1msmoke test passed\x1b[0m: the agent committed \"{subject}\", \
         finished its turn, and the session stopped\n"
    );
    Ok(())
}

/// What a session's events have shown so far.
#[derive(Default)]
struct Seen {
    after: u64,
    status: Option<StatusChange>,
    turn_ok: bool,
    commit: Option<CommitInfo>,
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
            EventBody::Status(s) => self.status = Some(s.clone()),
            EventBody::Claude(v) if e.body.claude_type() == Some("result") => {
                if v["is_error"].as_bool() == Some(true) {
                    bail!("the agent's turn failed: {v}");
                }
                self.turn_ok = true;
            }
            EventBody::Git(g) if g.ahead > 0 => self.commit = g.last_commit.clone(),
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
    let url = format!("{API}/api/v1{path}");
    let agent = agent();
    let mut req = ureq::http::Request::builder()
        .method(method)
        .uri(&url)
        .header("Origin", API)
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
        .find(|v| v.starts_with("__Host-fctl="))
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

/// Makes sure the dev owner, a token, a Claude credential and a project exist.
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
    ok(
        "signing in as the dev owner (delete .fctl-dev and crates/fctl-control/.wrangler to start over)",
        &res,
    )?;
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
         server = \"{API}\"\n\
         name = \"dev-host\"\n\
         driver = \"process\"\n\
         capacity = 4\n\
         claude_bin = {claude}\n\
         agent_bin = {agent}\n\
         data_dir = {data}\n\
         push_on_commit = false\n",
        claude = toml_str(&bin.join("fake-claude")),
        agent = toml_str(&bin.join("fctl-agent")),
        data = toml_str(&state.join("data")),
    );
    std::fs::write(config.join("host.toml"), toml)?;
    Command::new(bin.join("fctl"))
        .args(["host", "run"])
        .env("FCTL_CONFIG_DIR", &config)
        .env("FCTL_DATA_DIR", state.join("data"))
        .env(
            "RUST_LOG",
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
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
