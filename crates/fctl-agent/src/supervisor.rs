//! The supervisor: one actor that owns the session.
//!
//! Inputs: frames from the host, output from Claude, results of background
//! work (cloning, git state), and a clock. Outputs: journaled events, state
//! and ephemeral streams to the host, and stdin for Claude.

use crate::claude::{self, Claude, ClaudeOut, LaunchSpec};
use crate::journal::Journal;
use crate::link::{self, LinkEvent};
use crate::local;
use crate::prompt;
use crate::workspace::{self, Paths};
use anyhow::Result;
use fctl_domain::events::{self as ev, BillingGuard, ClaudeKind};
use fctl_domain::validate;
use fctl_proto::agent::{
    AgentConfig, AgentHello, AgentToHost, HostToAgent, SESSION_SOCKET_ENV, SessionReply,
    SessionRequest,
};
use fctl_proto::event::{EventBody, GitState, LogLevel, LogLine, LogSource, Notice, NoticeLevel};
use fctl_proto::runner::{
    ClaudeCredential, ClaudeCredentialKind, CredentialKind, GithubCredential, StopReason,
};
use fctl_proto::session::{RemoteControlState, SessionStatus};
use fctl_proto::uplink::{AgentEvent, Delivery, DeliveryKind, SessionState};
use fctl_proto::{ApiError, DeliveryId, PROTOCOL_VERSION, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// What survives a supervisor restart (no secrets).
#[derive(Default, Serialize, Deserialize)]
struct Persisted {
    claude_started: bool,
    delivered: VecDeque<String>,
    remote_control_url: Option<String>,
    /// The title and description the agent gave the session (`fctl edit`).
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

pub(crate) enum Work {
    Prepared(Result<(), String>),
    Git(GitState),
    Log(LogLevel, String, LogSource),
}

/// Why we asked the host for a credential.
enum CredFor {
    /// Answer Claude's `oauth_token_refresh` with this request id.
    ClaudeRefresh(String),
    GithubRefresh,
}

pub struct Supervisor {
    state_dir: PathBuf,
    /// Where programs in the sandbox reach us (`fctl edit`), told to Claude.
    session_socket: Option<PathBuf>,
    journal: Journal,
    persisted: Persisted,
    host: Option<(u64, mpsc::UnboundedSender<String>)>,
    config: Option<AgentConfig>,
    paths: Option<Paths>,
    state: SessionState,
    claude: Option<Claude>,
    claude_tx: mpsc::UnboundedSender<ClaudeOut>,
    work_tx: mpsc::UnboundedSender<Work>,
    /// Messages that arrived before Claude was ready.
    queue: VecDeque<Delivery>,
    turn_active: bool,
    stopping: Option<(StopReason, Instant)>,
    idle_since: Option<Instant>,
    resume_at: Option<i64>,
    github_expires_at: i64,
    pending_creds: HashMap<String, CredFor>,
    next_request: u64,
    outbox: Vec<AgentEvent>,
    restarts: VecDeque<Instant>,
    stderr_budget: (Instant, u32),
    claude_credential: Option<ClaudeCredential>,
    exit_after: Option<Instant>,
    /// The repositories' state after the last turn, for its pull requests.
    last_git: Option<GitState>,
    /// Said once that the sandbox's fctl cannot rename the session.
    told_no_fctl: bool,
}

impl Supervisor {
    pub fn new(
        state_dir: PathBuf,
        session_socket: Option<PathBuf>,
    ) -> Result<(
        Self,
        mpsc::UnboundedReceiver<ClaudeOut>,
        mpsc::UnboundedReceiver<Work>,
    )> {
        let journal = Journal::open(&state_dir)?;
        let persisted: Persisted = std::fs::read_to_string(state_dir.join("state.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let (claude_tx, claude_rx) = mpsc::unbounded_channel();
        let (work_tx, work_rx) = mpsc::unbounded_channel();
        Ok((
            Self {
                state_dir,
                session_socket,
                journal,
                host: None,
                config: None,
                paths: None,
                state: SessionState {
                    status: SessionStatus::Provisioning,
                    reason: None,
                    resume_at: None,
                    branch: None,
                    model: None,
                    claude_session_id: None,
                    remote_control: None,
                    stats: None,
                    title: persisted.title.clone(),
                    description: persisted.description.clone(),
                },
                persisted,
                claude: None,
                claude_tx,
                work_tx,
                queue: VecDeque::new(),
                turn_active: false,
                stopping: None,
                idle_since: None,
                resume_at: None,
                github_expires_at: i64::MAX,
                pending_creds: HashMap::new(),
                next_request: 0,
                outbox: Vec::new(),
                restarts: VecDeque::new(),
                stderr_budget: (Instant::now(), 0),
                claude_credential: None,
                exit_after: None,
                last_git: None,
                told_no_fctl: false,
            },
            claude_rx,
            work_rx,
        ))
    }

    pub async fn run(
        mut self,
        mut link_rx: mpsc::UnboundedReceiver<LinkEvent>,
        mut claude_rx: mpsc::UnboundedReceiver<ClaudeOut>,
        mut work_rx: mpsc::UnboundedReceiver<Work>,
        mut local_rx: mpsc::UnboundedReceiver<local::Call>,
    ) -> Result<()> {
        let mut flush = tokio::time::interval(Duration::from_millis(40));
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                Some(e) = link_rx.recv() => self.on_link(e).await,
                Some(out) = claude_rx.recv() => self.on_claude(out).await,
                Some(w) = work_rx.recv() => self.on_work(w).await,
                Some(call) = local_rx.recv() => {
                    let reply = self.on_session_request(call.request);
                    let _ = call.reply.send(reply);
                }
                _ = flush.tick() => self.flush(),
                _ = tick.tick() => {
                    self.on_tick().await;
                    if self.exit_after.is_some_and(|t| Instant::now() >= t) {
                        self.flush();
                        return Ok(());
                    }
                }
            }
        }
    }

    // ───────────────────────────── host link ─────────────────────────────

    fn send(&self, msg: &AgentToHost) {
        if let (Some((_, tx)), Some(text)) = (&self.host, link::encode(msg)) {
            let _ = tx.send(text);
        }
    }

    fn flush(&mut self) {
        if self.outbox.is_empty() || self.host.is_none() {
            return;
        }
        // Batches of at most 100 events.
        while !self.outbox.is_empty() {
            let n = self.outbox.len().min(100);
            let batch: Vec<AgentEvent> = self.outbox.drain(..n).collect();
            self.send(&AgentToHost::Events { events: batch });
        }
    }

    fn hello(&self) -> AgentToHost {
        AgentToHost::Hello(AgentHello {
            protocol: PROTOCOL_VERSION,
            version: VERSION.to_owned(),
            configured: self.config.is_some(),
            status: self.state.status,
            agent_seq: self.journal.last_seq(),
            claude_version: None,
        })
    }

    async fn on_link(&mut self, e: LinkEvent) {
        match e {
            LinkEvent::Connected(id, tx) => {
                self.host = Some((id, tx));
                let hello = self.hello();
                self.send(&hello);
            }
            LinkEvent::Disconnected(id) => {
                if self.host.as_ref().is_some_and(|(cur, _)| *cur == id) {
                    self.host = None;
                }
            }
            LinkEvent::Frame(id, frame) => {
                if self.host.as_ref().is_some_and(|(cur, _)| *cur == id) {
                    self.on_frame(frame).await;
                }
            }
        }
    }

    async fn on_frame(&mut self, frame: HostToAgent) {
        match frame {
            HostToAgent::Ping => self.send(&AgentToHost::Pong),
            HostToAgent::Configure { config } => {
                if self.config.is_none() {
                    self.configure(*config);
                } else {
                    self.log(
                        LogSource::Agent,
                        LogLevel::Debug,
                        "Already configured; ignoring a repeated Configure",
                    );
                }
            }
            HostToAgent::Replay { after_agent_seq } => {
                self.outbox.clear();
                match self.journal.read_after(after_agent_seq) {
                    Ok(events) => {
                        for chunk in events.chunks(100) {
                            self.send(&AgentToHost::Events {
                                events: chunk.to_vec(),
                            });
                        }
                    }
                    Err(e) => tracing::error!("journal replay failed: {e:#}"),
                }
                self.send(&AgentToHost::State {
                    state: self.state.clone(),
                });
            }
            HostToAgent::Ack { agent_seq } => {
                if let Err(e) = self.journal.ack(agent_seq) {
                    tracing::warn!("journal ack failed: {e:#}");
                }
            }
            HostToAgent::Deliver { delivery } => self.on_delivery(delivery).await,
            HostToAgent::Credentials {
                request_id,
                claude,
                github,
                github_read,
                error,
            } => {
                self.on_credentials(request_id, claude, github, github_read, error);
            }
            HostToAgent::Stop { reason, grace_ms } => self.begin_stop(reason, grace_ms).await,
        }
    }

    // ───────────────────────────── journaling and state ─────────────────────────────

    fn emit(&mut self, body: EventBody) {
        match self.journal.append(now_ms(), body) {
            Ok(ev) => self.outbox.push(ev),
            Err(e) => tracing::error!("journal write failed: {e:#}"),
        }
    }

    fn log(&mut self, source: LogSource, level: LogLevel, text: &str) {
        self.emit(EventBody::Log(LogLine {
            source,
            level,
            text: text.to_owned(),
        }));
    }

    fn notice(&mut self, level: NoticeLevel, code: &str, message: &str, until: Option<i64>) {
        self.emit(EventBody::Notice(Notice {
            level,
            code: code.into(),
            message: message.into(),
            until,
        }));
    }

    fn set_status(&mut self, status: SessionStatus, reason: Option<String>) {
        if self.state.status == status && self.state.reason == reason {
            return;
        }
        self.state.status = status;
        self.state.reason = reason;
        self.state.resume_at = if status == SessionStatus::Paused {
            self.resume_at
        } else {
            None
        };
        self.idle_since = (status == SessionStatus::Idle).then(Instant::now);
        self.flush();
        self.send(&AgentToHost::State {
            state: self.state.clone(),
        });
    }

    fn persist(&self) {
        if let Ok(text) = serde_json::to_string(&self.persisted) {
            let _ = std::fs::write(self.state_dir.join("state.json"), text);
        }
    }

    // ───────────────────────────── the session socket ─────────────────────────────

    fn on_session_request(&mut self, request: SessionRequest) -> SessionReply {
        match request {
            SessionRequest::Edit {
                session_id,
                title,
                description,
            } => self
                .edit_session(session_id, title, description)
                .unwrap_or_else(|error| SessionReply::Error { error }),
        }
    }

    /// The agent names or describes its session: the title and description
    /// travel with the session state, which the host passes on to the
    /// control plane (and sends again after every reconnect).
    fn edit_session(
        &mut self,
        session_id: Option<SessionId>,
        title: Option<String>,
        description: Option<String>,
    ) -> std::result::Result<SessionReply, ApiError> {
        let Some(spec) = self.config.as_ref().map(|c| c.spec.clone()) else {
            return Err(ApiError::new(
                fctl_proto::ErrorCode::Conflict,
                "The session is still starting; try again in a moment",
            ));
        };
        if let Some(id) = session_id.filter(|id| *id != spec.session_id) {
            return Err(ApiError::bad_request(format!(
                "This sandbox runs {}, not {id}",
                spec.session_id
            )));
        }
        if title.is_none() && description.is_none() {
            return Err(ApiError::bad_request("Give a title, a description or both"));
        }
        let title = title.as_deref().map(validate::session_title).transpose()?;
        let description = description
            .as_deref()
            .map(validate::session_description)
            .transpose()?;
        if let Some(t) = title {
            self.state.title = Some(t);
        }
        // An empty description clears the one the session has.
        if let Some(d) = description {
            self.state.description = Some(d.unwrap_or_default());
        }
        self.persisted.title = self.state.title.clone();
        self.persisted.description = self.state.description.clone();
        self.persist();
        self.flush();
        self.send(&AgentToHost::State {
            state: self.state.clone(),
        });
        Ok(SessionReply::Edited {
            id: spec.session_id.clone(),
            project_slug: spec.project.slug.clone(),
            number: spec.number,
            title: self.state.title.clone().unwrap_or(spec.title),
            description: self.state.description.clone().filter(|d| !d.is_empty()),
        })
    }

    fn mark_delivered(&mut self, id: &DeliveryId) {
        self.persisted.delivered.push_back(id.0.clone());
        while self.persisted.delivered.len() > 2000 {
            self.persisted.delivered.pop_front();
        }
        self.persist();
    }

    fn was_delivered(&self, id: &DeliveryId) -> bool {
        self.persisted.delivered.iter().any(|d| d == &id.0)
    }

    // ───────────────────────────── preparation ─────────────────────────────

    fn configure(&mut self, config: AgentConfig) {
        if let Some(seq) = config.first_agent_seq
            && let Err(e) = self.journal.start_after(seq)
        {
            tracing::error!("journal: {e:#}");
        }
        let home = config
            .home
            .clone()
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/root"));
        // `~/workspace` (container sandboxes) is relative to HOME.
        let workdir = match config.workdir.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(&config.workdir),
        };
        let mut config = config;
        config.workdir = workdir.display().to_string();
        let paths = Paths::new(&workdir, &home, &self.state_dir);
        self.state.branch = Some(config.spec.branch.clone());
        self.state.claude_session_id = Some(config.spec.claude_session_id.clone());
        let creds = &config.spec.credentials;
        if let Some(at) = expiry(creds.github.as_ref(), creds.github_read.as_ref()) {
            self.github_expires_at = at;
        }
        self.claude_credential = Some(config.spec.credentials.claude.clone());
        self.paths = Some(paths.clone());
        let spec = config.spec.clone();
        let sandbox_env = prompt::sandbox_env(config.sandbox);
        self.config = Some(config);
        self.set_status(
            SessionStatus::Preparing,
            Some("Preparing the workspace".into()),
        );
        let repos = spec.project.repos.as_deref().unwrap_or_default();
        if !repos.is_empty() {
            self.log(
                LogSource::Agent,
                LogLevel::Info,
                &prompt::repos_line(repos, &paths.repos.display().to_string()),
            );
        }
        for r in repos {
            if let Some(why) = &r.unavailable {
                self.notice(NoticeLevel::Warn, "repo_unavailable", why, None);
            }
        }
        let work = self.work_tx.clone();
        tokio::spawn(async move {
            let (log_tx, mut log_rx) = mpsc::unbounded_channel::<(LogLevel, String)>();
            let fwd_work = work.clone();
            let forward = tokio::spawn(async move {
                while let Some((level, text)) = log_rx.recv().await {
                    let _ = fwd_work.send(Work::Log(level, text, LogSource::Agent));
                }
            });
            let result = async {
                workspace::configure_git(&paths, &spec.git_author).await?;
                workspace::write_routes(&paths, &spec)?;
                workspace::write_github_tokens(
                    &paths,
                    spec.credentials.github.as_ref(),
                    spec.credentials.github_read.as_ref(),
                )?;
                workspace::prepare(&paths, &spec, &log_tx).await?;
                if let Some(script) = spec
                    .project
                    .settings
                    .setup_script
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                {
                    let env: Vec<(String, String)> = spec
                        .project
                        .settings
                        .env
                        .iter()
                        .chain(spec.secrets.iter())
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .chain(sandbox_env)
                        .collect();
                    let (setup_tx, mut setup_rx) = mpsc::unbounded_channel::<(LogLevel, String)>();
                    let setup_work = work.clone();
                    let setup_fwd = tokio::spawn(async move {
                        while let Some((level, text)) = setup_rx.recv().await {
                            let _ = setup_work.send(Work::Log(level, text, LogSource::Setup));
                        }
                    });
                    let ok = workspace::setup_script(&paths, script, &env, &setup_tx).await?;
                    drop(setup_tx);
                    let _ = setup_fwd.await;
                    if !ok {
                        let _ = log_tx.send((
                            LogLevel::Warn,
                            "The setup script failed; continuing so the agent can fix it".into(),
                        ));
                    }
                }
                anyhow::Ok(())
            }
            .await;
            drop(log_tx);
            let _ = forward.await;
            let _ = work.send(Work::Prepared(result.map_err(|e| format!("{e:#}"))));
        });
    }

    async fn on_work(&mut self, w: Work) {
        match w {
            Work::Log(level, text, source) => self.log(source, level, &text),
            Work::Prepared(Ok(())) => {
                if let Err(e) = self.start_claude().await {
                    let msg = format!("Claude could not start: {e:#}");
                    self.notice(NoticeLevel::Error, "claude_start_failed", &msg, None);
                    self.set_status(SessionStatus::Failed, Some(msg));
                    self.finish().await;
                }
            }
            Work::Prepared(Err(e)) => {
                self.notice(NoticeLevel::Error, "prepare_failed", &e, None);
                self.set_status(SessionStatus::Failed, Some(e));
                self.finish().await;
            }
            Work::Git(state) => {
                if let Some(pr) = &state.pull_request {
                    tracing::info!("pull request #{} {}", pr.number, pr.url);
                }
                self.last_git = Some(state.clone());
                self.emit(EventBody::Git(state));
            }
        }
    }

    // ───────────────────────────── Claude ─────────────────────────────

    fn write_claude_config(
        &self,
        home: &std::path::Path,
        workdir: &str,
        claude_ai_connectors: bool,
    ) -> Result<()> {
        let claude_dir = home.join(".claude");
        std::fs::create_dir_all(&claude_dir)?;
        let path = home.join(".claude.json");
        let mut cfg: Value = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| json!({}));
        cfg["hasCompletedOnboarding"] = json!(true);
        cfg["bypassPermissionsModeAccepted"] = json!(true);
        cfg["autoUpdates"] = json!(false);
        cfg["projects"][workdir]["hasTrustDialogAccepted"] = json!(true);
        cfg["projects"][workdir]["hasCompletedProjectOnboarding"] = json!(true);
        if let Some(raw) = self
            .claude_credential
            .as_ref()
            .and_then(|c| c.account.as_ref())
            .and_then(|a| a.raw.clone())
        {
            cfg["oauthAccount"] = raw;
        }
        std::fs::write(&path, serde_json::to_string_pretty(&cfg)?)?;
        std::fs::write(
            claude_dir.join("settings.json"),
            serde_json::to_string_pretty(&claude::user_settings(claude_ai_connectors))?,
        )?;
        Ok(())
    }

    async fn start_claude(&mut self) -> Result<()> {
        let (Some(config), Some(paths)) = (self.config.clone(), self.paths.clone()) else {
            anyhow::bail!("not configured");
        };
        let spec = &config.spec;
        let workdir = config.workdir.clone();
        self.write_claude_config(
            &paths.home,
            &workdir,
            spec.project.settings.claude_ai_connectors,
        )?;
        if self.session_socket.is_some() && fctl_can_edit(&paths).await {
            if let Err(e) = claude::write_instructions(&paths.home, prompt::AGENTS_MD) {
                tracing::warn!("writing ~/AGENTS.md failed: {e:#}");
            }
        } else if !self.told_no_fctl {
            self.told_no_fctl = true;
            self.log(
                LogSource::Agent,
                LogLevel::Info,
                "This sandbox has no fctl that can rename the session, so it keeps the title it started with. Rebuilding the default image (fctl host image build) installs one.",
            );
        }
        let setting_sources = match claude::project_settings_are_unsafe(&paths.workdir) {
            Some(why) => {
                let msg = format!(
                    "Ignoring the repository's Claude settings because {why}: fctl only runs Claude on subscriptions."
                );
                self.notice(NoticeLevel::Warn, "project_settings_ignored", &msg, None);
                "user"
            }
            None => "user,project,local",
        };
        let cred = self
            .claude_credential
            .clone()
            .unwrap_or_else(|| spec.credentials.claude.clone());
        let project_env = fctl_domain::settings::session_env(&spec.project.settings);
        let mut env = std::collections::BTreeMap::new();
        for (k, v) in project_env
            .iter()
            .chain(spec.secrets.iter())
            .chain(config.env.iter())
        {
            env.insert(k.clone(), v.clone());
        }
        env.insert("CLAUDE_CODE_OAUTH_TOKEN".into(), cred.access_token.clone());
        env.insert(
            "CLAUDE_CODE_OAUTH_SCOPES".into(),
            if cred.scopes.is_empty() {
                "user:inference".into()
            } else {
                cred.scopes.join(" ")
            },
        );
        if let Some(t) = &cred.subscription_type {
            env.insert("CLAUDE_CODE_SUBSCRIPTION_TYPE".into(), t.clone());
        }
        env.insert("IS_SANDBOX".into(), "1".into());
        env.insert("DISABLE_AUTOUPDATER".into(), "1".into());
        env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
        env.insert("FCTL_SESSION_ID".into(), spec.session_id.0.clone());
        env.insert("FCTL_SESSION_URL".into(), spec.web_url.clone());
        if let Some(socket) = &self.session_socket {
            env.insert(SESSION_SOCKET_ENV.into(), socket.display().to_string());
        }
        env.extend(prompt::sandbox_env(config.sandbox));
        // Resuming needs Claude's conversation. A sandbox made for a resumed
        // session (the stopped one expired) has none: start a new one there.
        let wants_resume = spec.resume || self.persisted.claude_started;
        let resume = wants_resume
            && (self.persisted.claude_started
                || claude::conversation_exists(&paths.home, &spec.claude_session_id));
        let fresh_resume = wants_resume && !resume;
        let system = prompt::system_prompt(spec, &workdir, &paths, config.sandbox);
        let bin = config.claude_bin.clone().unwrap_or_else(|| "claude".into());
        let launch = LaunchSpec {
            bin: &bin,
            cwd: &paths.workdir,
            home: &paths.home,
            claude_session_id: &spec.claude_session_id,
            resume,
            model: spec
                .model
                .as_deref()
                .or(spec.project.settings.model.as_deref()),
            effort: spec
                .effort
                .as_deref()
                .or(spec.project.settings.effort.as_deref()),
            append_system_prompt: &system,
            setting_sources,
            extra_args: &config.claude_args,
            env,
        };
        let mut c = Claude::spawn(launch, self.claude_tx.clone())?;
        self.log(
            LogSource::Agent,
            LogLevel::Info,
            &format!("Started Claude Code (pid {})", c.pid().unwrap_or(0)),
        );
        let init = c
            .request(json!({ "subtype": "initialize" }), Duration::from_secs(90))
            .await;
        if let Err(e) = init {
            c.kill().await;
            anyhow::bail!("Claude did not initialize: {e}");
        }
        self.persisted.claude_started = true;
        self.persist();
        // Remote Control: follow the session in claude.ai/code and the apps.
        if spec.project.settings.remote_control {
            let rc = if cred.kind == ClaudeCredentialKind::Oauth
                && cred.scopes.iter().any(|s| s == "user:profile")
            {
                let name = format!("{} · {}", spec.project.name, spec.title);
                match c
                    .request(
                        json!({ "subtype": "remote_control", "enabled": true, "name": name }),
                        Duration::from_secs(45),
                    )
                    .await
                {
                    Ok(v) => RemoteControlState {
                        enabled: true,
                        session_url: v
                            .get("session_url")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        unavailable_reason: None,
                    },
                    Err(e) => RemoteControlState {
                        enabled: false,
                        session_url: None,
                        unavailable_reason: Some(e),
                    },
                }
            } else {
                RemoteControlState {
                    enabled: false,
                    session_url: None,
                    unavailable_reason: Some(
                        "Remote Control needs a full Claude login. Connect one under Settings → Claude.".into(),
                    ),
                }
            };
            self.persisted.remote_control_url = rc.session_url.clone();
            self.persist();
            self.state.remote_control = Some(rc.clone());
            self.emit(EventBody::RemoteControl(rc));
        }
        self.claude = Some(c);
        // The first turn, then anything that queued up meanwhile.
        if fresh_resume {
            self.log(
                LogSource::Agent,
                LogLevel::Info,
                "The earlier conversation went with the old sandbox; Claude starts a new one on the branch",
            );
            let other_repos = spec.project.repos.as_ref().is_some_and(|r| !r.is_empty());
            let text = prompt::fresh_resume(&spec.branch, spec.task.as_deref(), other_repos);
            self.send_turn(&text, None, None).await;
        } else if !resume && let Some(p) = spec.prompt.clone() {
            self.send_turn(&p, None, None).await;
        }
        if self.turn_active {
            self.set_status(SessionStatus::Running, None);
        } else {
            self.set_status(SessionStatus::Idle, Some("Waiting for a message".into()));
        }
        while let Some(d) = self.queue.pop_front() {
            self.on_delivery(d).await;
        }
        Ok(())
    }

    async fn send_turn(&mut self, text: &str, author: Option<&str>, delivery: Option<&DeliveryId>) {
        let starter = self
            .config
            .as_ref()
            .map(|c| c.spec.author.name.clone())
            .unwrap_or_default();
        let content = prompt::user_turn(text, author, &starter);
        if let Some(c) = &self.claude {
            c.send_user(&content, &fctl_domain::ids::uuid_v4());
            self.turn_active = true;
            self.idle_since = None;
            if let Some(id) = delivery {
                self.mark_delivered(id);
                self.send(&AgentToHost::Delivered {
                    delivery_id: id.clone(),
                    ok: true,
                    error: None,
                });
            }
            if self.state.status != SessionStatus::Running {
                self.set_status(SessionStatus::Running, None);
            }
        }
    }

    async fn on_delivery(&mut self, d: Delivery) {
        if self.was_delivered(&d.id) {
            self.send(&AgentToHost::Delivered {
                delivery_id: d.id,
                ok: true,
                error: None,
            });
            return;
        }
        match &d.kind {
            DeliveryKind::Message { text, author } => {
                if self.stopping.is_some() {
                    // Leave it unconfirmed: the control plane keeps it for the next run.
                    return;
                }
                if self.claude.is_none() {
                    self.queue.push_back(d.clone());
                    return;
                }
                let (text, author) = (text.clone(), author.as_ref().map(|a| a.name.clone()));
                // A message during a usage-limit pause waits for the reset.
                if self.state.status == SessionStatus::Paused {
                    self.queue.push_back(d.clone());
                    return;
                }
                self.send_turn(&text, author.as_deref(), Some(&d.id)).await;
            }
            DeliveryKind::Interrupt { .. } => {
                if let Some(c) = self.claude.as_mut()
                    && self.turn_active
                {
                    c.request_nowait(json!({ "subtype": "interrupt" }));
                }
                self.queue
                    .retain(|q| !matches!(q.kind, DeliveryKind::Message { .. }));
                self.mark_delivered(&d.id);
                self.send(&AgentToHost::Delivered {
                    delivery_id: d.id,
                    ok: true,
                    error: None,
                });
            }
        }
    }

    async fn on_claude(&mut self, out: ClaudeOut) {
        match out {
            ClaudeOut::Message(v) => self.on_claude_message(v).await,
            ClaudeOut::ControlRequest {
                request_id,
                request,
            } => self.on_claude_request(request_id, request),
            ClaudeOut::Stderr(line) => {
                // At most 60 stderr lines a minute reach the transcript.
                let (since, n) = self.stderr_budget;
                if since.elapsed() > Duration::from_secs(60) {
                    self.stderr_budget = (Instant::now(), 0);
                }
                if self.stderr_budget.1 < 60 {
                    self.stderr_budget.1 += 1;
                    self.log(LogSource::Claude, LogLevel::Warn, &line);
                } else if n == 60 {
                    self.stderr_budget.1 += 1;
                    self.log(
                        LogSource::Claude,
                        LogLevel::Warn,
                        "(more stderr output suppressed)",
                    );
                }
            }
            ClaudeOut::Exited(code) => self.on_claude_exit(code).await,
        }
    }

    async fn on_claude_message(&mut self, v: Value) {
        let kind = ev::classify(&v);
        if ev::is_ephemeral(kind) {
            self.send(&AgentToHost::Stream { event: v });
            return;
        }
        if let Some(guard) = ev::billing_guard(&v) {
            self.emit(EventBody::Claude(v.clone()));
            self.on_billing_guard(guard).await;
            return;
        }
        if kind == ClaudeKind::Init
            && let Some(m) = v.get("model").and_then(Value::as_str)
        {
            self.state.model = Some(m.to_owned());
        }
        let is_result = kind == ClaudeKind::Result;
        let more_queued = v
            .get("queued_turn_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0;
        self.emit(EventBody::Claude(v));
        if is_result {
            self.turn_active = more_queued;
            if !more_queued
                && self.stopping.is_none()
                && self.state.status == SessionStatus::Running
            {
                self.set_status(SessionStatus::Idle, Some("Waiting for a message".into()));
            }
            self.after_turn();
        }
    }

    /// After each turn: git state, pushes and pull requests, in every
    /// repository.
    fn after_turn(&mut self) {
        let (Some(config), Some(paths)) = (self.config.clone(), self.paths.clone()) else {
            return;
        };
        let work = self.work_tx.clone();
        tokio::spawn(async move {
            let Ok(synced) =
                workspace::sync(&paths, &config.spec, config.push_on_commit, true).await
            else {
                return;
            };
            for (level, text) in synced.logs {
                let _ = work.send(Work::Log(level, text, LogSource::Agent));
            }
            let _ = work.send(Work::Git(synced.state));
        });
    }

    async fn on_billing_guard(&mut self, guard: BillingGuard) {
        match guard {
            BillingGuard::ApiKey { source } => {
                let msg = format!(
                    "Claude reported an API key credential ({source}). fctl only runs Claude on subscriptions, so the session was stopped before it could bill an API account."
                );
                self.notice(NoticeLevel::Error, "api_key_refused", &msg, None);
                if let Some(mut c) = self.claude.take() {
                    c.kill().await;
                }
                self.set_status(SessionStatus::Failed, Some(msg));
                self.finish().await;
            }
            BillingGuard::Overage => {
                if let Some(c) = self.claude.as_mut() {
                    c.request_nowait(json!({ "subtype": "interrupt" }));
                }
                let until = now_ms() + 60 * 60 * 1000;
                self.resume_at = Some(until);
                self.notice(
                    NoticeLevel::Warn,
                    "overage_blocked",
                    "Claude started using paid extra usage, so fctl interrupted it. The session continues when the subscription limit resets.",
                    Some(until),
                );
                self.set_status(
                    SessionStatus::Paused,
                    Some("Paused to avoid paid extra usage".into()),
                );
            }
            BillingGuard::LimitReached { resets_at, limit } => {
                let until = resets_at.unwrap_or(now_ms() + 60 * 60 * 1000) + 30_000;
                self.resume_at = Some(until);
                let which = limit.unwrap_or_else(|| "usage".into()).replace('_', "-");
                self.notice(
                    NoticeLevel::Info,
                    "usage_limit",
                    &format!("The Claude {which} limit is reached. fctl continues the session when it resets."),
                    Some(until),
                );
                self.set_status(
                    SessionStatus::Paused,
                    Some("Waiting for the usage limit to reset".into()),
                );
            }
        }
    }

    fn on_claude_request(&mut self, request_id: String, request: Value) {
        let subtype = request
            .get("subtype")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match subtype {
            "oauth_token_refresh" => {
                let rid = self.request_id();
                self.pending_creds
                    .insert(rid.clone(), CredFor::ClaudeRefresh(request_id));
                self.send(&AgentToHost::CredentialRequest {
                    request_id: rid,
                    kind: CredentialKind::Claude,
                });
            }
            "can_use_tool" => {
                let input = request.get("input").cloned().unwrap_or_else(|| json!({}));
                if let Some(c) = &self.claude {
                    c.respond(
                        &request_id,
                        Ok(json!({ "behavior": "allow", "updatedInput": input })),
                    );
                }
            }
            other => {
                if let Some(c) = &self.claude {
                    c.respond(
                        &request_id,
                        Err(format!("fctl-agent does not handle {other}")),
                    );
                }
            }
        }
    }

    fn request_id(&mut self) -> String {
        self.next_request += 1;
        format!("cred_{}", self.next_request)
    }

    fn on_credentials(
        &mut self,
        request_id: Option<String>,
        claude: Option<ClaudeCredential>,
        github: Option<GithubCredential>,
        github_read: Option<GithubCredential>,
        error: Option<String>,
    ) {
        if let (Some(at), Some(paths)) =
            (expiry(github.as_ref(), github_read.as_ref()), &self.paths)
        {
            self.github_expires_at = at;
            if let Err(e) =
                workspace::write_github_tokens(paths, github.as_ref(), github_read.as_ref())
            {
                tracing::warn!("writing the GitHub tokens failed: {e:#}");
            }
        }
        if let Some(c) = &claude {
            self.claude_credential = Some(c.clone());
        }
        let Some(rid) = request_id else { return };
        match self.pending_creds.remove(&rid) {
            Some(CredFor::ClaudeRefresh(claude_req)) => {
                let answer = match (claude, error) {
                    (Some(c), _) => Ok(json!({ "accessToken": c.access_token })),
                    (None, Some(e)) => {
                        self.notice(
                            NoticeLevel::Error,
                            "credential_expired",
                            &format!("Claude's credential could not be refreshed: {e}"),
                            None,
                        );
                        Ok(json!({ "accessToken": null, "reason": "host_declined" }))
                    }
                    (None, None) => Ok(json!({ "accessToken": null })),
                };
                if let Some(c) = &self.claude {
                    c.respond(&claude_req, answer);
                }
            }
            Some(CredFor::GithubRefresh) => {
                if let Some(e) = error {
                    self.log(
                        LogSource::Agent,
                        LogLevel::Warn,
                        &format!("GitHub token refresh failed: {e}"),
                    );
                }
            }
            None => {}
        }
    }

    async fn on_claude_exit(&mut self, _code: Option<i32>) {
        let code = match self.claude.take() {
            Some(mut c) => tokio::time::timeout(Duration::from_secs(5), c.wait())
                .await
                .ok()
                .flatten(),
            None => return,
        };
        self.turn_active = false;
        if self.stopping.is_some() || self.state.status.is_terminal() {
            return;
        }
        let now = Instant::now();
        self.restarts
            .retain(|t| now.duration_since(*t) < Duration::from_secs(600));
        if self.restarts.len() >= 3 {
            let msg = format!(
                "Claude exited (code {}) three times in ten minutes",
                code.map_or("?".into(), |c| c.to_string())
            );
            self.notice(NoticeLevel::Error, "claude_crashed", &msg, None);
            self.set_status(SessionStatus::Failed, Some(msg));
            self.finish().await;
            return;
        }
        self.restarts.push_back(now);
        self.notice(
            NoticeLevel::Warn,
            "claude_restarted",
            &format!(
                "Claude exited unexpectedly (code {}); resuming the conversation",
                code.map_or("?".into(), |c| c.to_string())
            ),
            None,
        );
        if let Err(e) = self.start_claude().await {
            let msg = format!("Claude could not restart: {e:#}");
            self.set_status(SessionStatus::Failed, Some(msg));
            self.finish().await;
        }
    }

    // ───────────────────────────── timers and stopping ─────────────────────────────

    async fn on_tick(&mut self) {
        let now = now_ms();
        // Usage limit reset: continue.
        if self.state.status == SessionStatus::Paused && self.resume_at.is_some_and(|t| now >= t) {
            self.resume_at = None;
            self.set_status(
                SessionStatus::Running,
                Some("Resuming after the usage limit reset".into()),
            );
            self.send_turn(
                "The usage limit has reset. Continue where you left off.",
                None,
                None,
            )
            .await;
            while let Some(d) = self.queue.pop_front() {
                self.on_delivery(d).await;
            }
        }
        // GitHub tokens last an hour; refresh both ten minutes early.
        if self.github_expires_at - now < 10 * 60 * 1000 && self.host.is_some() {
            self.github_expires_at = now + 5 * 60 * 1000; // retry in five minutes if this fails
            let rid = self.request_id();
            self.pending_creds
                .insert(rid.clone(), CredFor::GithubRefresh);
            self.send(&AgentToHost::CredentialRequest {
                request_id: rid,
                kind: CredentialKind::Github,
            });
        }
        // Idle timeout.
        if let (Some(since), Some(cfg)) = (self.idle_since, self.config.as_ref())
            && since.elapsed() > Duration::from_secs(cfg.spec.idle_timeout_secs.max(60))
            && self.stopping.is_none()
        {
            self.begin_stop(StopReason::Idle, 30_000).await;
        }
        // A stop whose grace period ran out.
        if let Some((_, deadline)) = self.stopping
            && Instant::now() >= deadline
            && self.exit_after.is_none()
        {
            self.finish().await;
        }
    }

    async fn begin_stop(&mut self, reason: StopReason, grace_ms: u64) {
        if self.stopping.is_some() {
            return;
        }
        let deadline = Instant::now() + Duration::from_millis(grace_ms.clamp(1_000, 120_000));
        self.stopping = Some((reason, deadline));
        self.set_status(SessionStatus::Stopping, Some(stop_text(reason).into()));
        if self.turn_active {
            if let Some(c) = self.claude.as_mut() {
                c.request_nowait(json!({ "subtype": "interrupt" }));
            }
        } else {
            self.finish().await;
        }
    }

    /// Stops Claude, records the final repository state, and exits soon.
    async fn finish(&mut self) {
        if self.exit_after.is_some() {
            return;
        }
        if let Some(mut c) = self.claude.take() {
            c.kill().await;
        }
        if let (Some(config), Some(paths)) = (self.config.clone(), self.paths.clone())
            && let Ok(synced) =
                workspace::sync(&paths, &config.spec, config.push_on_commit, false).await
        {
            for (level, text) in synced.logs {
                self.log(LogSource::Agent, level, &text);
            }
            // No lookups while stopping: the pull requests found after the
            // last turn still stand.
            let mut st = synced.state;
            if let Some(last) = &self.last_git {
                workspace::keep_pull_requests(&mut st, last);
            }
            self.emit(EventBody::Git(st));
        }
        let (status, reason) = match self.stopping {
            Some((r, _)) => (SessionStatus::Stopped, Some(stop_text(r).to_owned())),
            None if self.state.status == SessionStatus::Failed => {
                (SessionStatus::Failed, self.state.reason.clone())
            }
            None => (SessionStatus::Stopped, Some("Stopped".into())),
        };
        self.set_status(status, reason.clone());
        self.flush();
        self.send(&AgentToHost::Exited { status, reason });
        // Give the host a moment to read everything.
        self.exit_after = Some(Instant::now() + Duration::from_secs(3));
    }
}

/// When the first of a session's GitHub tokens expires.
fn expiry(write: Option<&GithubCredential>, read: Option<&GithubCredential>) -> Option<i64> {
    write.iter().chain(read.iter()).map(|c| c.expires_at).min()
}

/// Whether the sandbox's `fctl` can name the session: custom images may have
/// none, and images built before `fctl edit` an older one.
async fn fctl_can_edit(paths: &Paths) -> bool {
    let help = workspace::run(&paths.home, &paths.home, "fctl", &["edit", "--help"]);
    matches!(
        tokio::time::timeout(Duration::from_secs(10), help).await,
        Ok(Ok(out)) if out.ok
    )
}

fn stop_text(r: StopReason) -> &'static str {
    match r {
        StopReason::User => "Stopped",
        StopReason::Idle => "Stopped after being idle",
        StopReason::Deleted => "Stopped because the session was deleted",
        StopReason::Shutdown => "Stopped because the host is shutting down",
    }
}

/// Used by `main` to run the supervisor and its sockets.
pub async fn serve(socket: PathBuf, state_dir: PathBuf) -> Result<()> {
    let listener = link::bind(&socket).await?;
    let (link_tx, link_rx) = mpsc::unbounded_channel();
    tokio::spawn(link::accept_loop(listener, link_tx));
    // Without the session socket the agent cannot rename the session, but
    // the session still runs.
    let session_socket = local::socket_path(&socket);
    let (local_tx, local_rx) = mpsc::unbounded_channel();
    let session_socket = match link::bind(&session_socket).await {
        Ok(listener) => {
            tokio::spawn(local::accept_loop(listener, local_tx));
            Some(session_socket)
        }
        Err(e) => {
            tracing::warn!("no session socket: {e:#}");
            None
        }
    };
    let (sup, claude_rx, work_rx) = Supervisor::new(state_dir, session_socket.clone())?;
    tracing::info!("fctl-agent {VERSION} serving {}", socket.display());
    let r = sup.run(link_rx, claude_rx, work_rx, local_rx).await;
    let _ = std::fs::remove_file(&socket);
    if let Some(s) = session_socket {
        let _ = std::fs::remove_file(s);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(title: Option<&str>, description: Option<&str>) -> SessionRequest {
        SessionRequest::Edit {
            session_id: None,
            title: title.map(str::to_owned),
            description: description.map(str::to_owned),
        }
    }

    #[tokio::test]
    async fn the_agent_names_and_describes_its_session() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let state_dir = dir.path().join("state");
        let (mut sup, _claude, _work) = Supervisor::new(state_dir.clone(), None)?;
        let early = sup.on_session_request(edit(Some("Too early"), None));
        assert!(matches!(early, SessionReply::Error { .. }));

        sup.config = Some(AgentConfig {
            spec: crate::workspace::tests::spec()?,
            workdir: dir.path().join("workspace").display().to_string(),
            home: None,
            claude_bin: None,
            claude_args: vec![],
            env: Default::default(),
            push_on_commit: true,
            first_agent_seq: None,
            sandbox: None,
        });
        let reply = sup.on_session_request(edit(
            Some("  Fix the   login test "),
            Some("Retries the redirect.\nAdds a test."),
        ));
        let SessionReply::Edited {
            id,
            number,
            title,
            description,
            ..
        } = reply
        else {
            panic!("expected Edited, got {reply:?}");
        };
        assert_eq!((id.as_str(), number), ("ses_test", 1));
        assert_eq!(title, "Fix the login test");
        assert_eq!(
            description.as_deref(),
            Some("Retries the redirect. Adds a test.")
        );
        assert_eq!(sup.state.title.as_deref(), Some("Fix the login test"));

        // An empty description clears it; the title stays.
        let cleared = sup.on_session_request(edit(None, Some("")));
        assert!(matches!(
            cleared,
            SessionReply::Edited { ref title, description: None, .. } if title == "Fix the login test"
        ));
        assert_eq!(sup.state.description.as_deref(), Some(""));

        // A restarted supervisor keeps reporting them.
        let (again, _claude, _work) = Supervisor::new(state_dir, None)?;
        assert_eq!(again.state.title.as_deref(), Some("Fix the login test"));

        let invalid = sup.on_session_request(edit(Some(" "), None));
        assert!(
            matches!(invalid, SessionReply::Error { ref error } if error.field.as_deref() == Some("title"))
        );
        let elsewhere = sup.on_session_request(SessionRequest::Edit {
            session_id: Some(SessionId("ses_other".into())),
            title: Some("Not mine".into()),
            description: None,
        });
        assert!(matches!(elsewhere, SessionReply::Error { .. }));
        assert_eq!(sup.state.title.as_deref(), Some("Fix the login test"));
        Ok(())
    }
}
