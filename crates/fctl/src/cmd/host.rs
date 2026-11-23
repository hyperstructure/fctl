//! `fctl host …`: make this machine a runner, and look after it there.

use crate::cmd::admin;
use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::Subcommand;
use clap::builder::BoolishValueParser;
use fctl_domain::{disk, image, size};
use fctl_host::config::{NOT_A_RUNNER, means_default_image};
use fctl_host::docker_access::{self, Problem};
use fctl_host::driver::{DiskSpot, Driver};
use fctl_host::{HostConfig, caches, claude_token, refresh, service, update};
use fctl_proto::api::{
    EnrollRunnerRequest, EnrollmentInfo, Runner, RunnerAutopilot, RunnerDetail, RunnerFacts,
    RunnerStatus,
};
use fctl_proto::runner::DriverInfo;
use std::io::IsTerminal;

#[derive(Subcommand)]
pub enum HostCmd {
    /// Register this machine as a runner and start the host service on it:
    /// signed in, for a workspace you are in; with a runner token; or,
    /// without signing in, with a workspace's enrollment token (`--enroll`).
    #[command(after_help = "Examples:
  fctl host install --workspace acme              # signed in (fctl login)
  fctl host install --enroll fctl_et_…            # as one of a workspace's runners, without signing in
  FCTL_SERVER=https://app.fctl.sh FCTL_ENROLL_TOKEN=fctl_et_… fctl host install   # from a boot script
  fctl host install --token fctl_rt_…             # with a runner token from a Runners tab")]
    Install {
        /// Runner name (default: the hostname).
        #[arg(long)]
        name: Option<String>,
        /// apple, docker, firecracker, lima, process, or auto.
        #[arg(long, default_value = "auto")]
        driver: String,
        /// Maximum concurrent sessions.
        #[arg(long, default_value_t = 4)]
        capacity: u32,
        /// CPUs for a sandbox whose project sets none (new runners: 4).
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        cpus: Option<u32>,
        /// Memory for a sandbox whose project sets none, like 8g or 8192m (new runners: 8g).
        #[arg(long, value_parser = memory_arg)]
        memory: Option<u32>,
        /// The most CPUs one sandbox may have (new runners: all of the machine's or the Docker VM's).
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        max_cpus: Option<u32>,
        /// The most memory one sandbox may have, like 16g (new runners: all of the machine's or the Docker VM's).
        #[arg(long, value_parser = memory_arg)]
        max_memory: Option<u32>,
        /// Sandbox image for sessions whose project sets none (default: the
        /// published image of this version; "" goes back to it).
        #[arg(long)]
        image: Option<String>,
        /// An image this runner runs for projects that choose their own: a
        /// reference, or a prefix like ghcr.io/acme/* (repeat it for more;
        /// it replaces the list, and "*" allows any). Without the list, any
        /// image runs.
        #[arg(long = "allow-image", value_name = "REF")]
        allow_images: Vec<String>,
        /// A repository, owner/name or owner/*, whose Dockerfile or
        /// devcontainer.json this runner builds images from (repeat it for
        /// more; it replaces the list, and "" builds none).
        #[arg(long = "allow-build", value_name = "REPO")]
        allow_builds: Vec<String>,
        /// The workspace whose sessions it runs (default from `fctl
        /// workspaces use`, else the one you administer, else the one you
        /// are in). Add it to more with `fctl workspaces add-runner`.
        #[arg(short, long, conflicts_with = "token")]
        workspace: Option<String>,
        /// Run only your sessions in the workspace. A member's runner
        /// always does; an admin's runs everyone's without it.
        #[arg(long, conflicts_with = "token")]
        personal: bool,
        /// Use an existing runner token instead of registering a new runner.
        #[arg(long, env = "FCTL_RUNNER_TOKEN", hide_env_values = true)]
        token: Option<String>,
        /// Enroll this machine with a workspace's enrollment token
        /// (fctl_et_…), without signing in: it becomes one of the
        /// workspace's runners, with a runner token of its own, and runs
        /// everyone's sessions there, or, with a token someone made for
        /// their own sessions, becomes their runner and runs only theirs;
        /// once approved if the token says so. Run again on a machine that
        /// enrolled the same way, it keeps that runner.
        #[arg(
            long,
            env = "FCTL_ENROLL_TOKEN",
            hide_env_values = true,
            value_name = "TOKEN",
            conflicts_with_all = ["token", "workspace", "personal"]
        )]
        enroll: Option<String>,
        /// Write the configuration but do not install a service.
        #[arg(long)]
        no_service: bool,
        /// Also build the sandbox image here (the Lima base VM, or for a runner
        /// that can't pull the published image).
        #[arg(long)]
        build_image: bool,
        /// on or off: whether the host updates fctl to the version the
        /// control plane runs, by itself (new runners: on).
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        auto_update: Option<bool>,
        /// on or off: whether fctl-agent runs as root in container and VM
        /// sandboxes, where the agent's commands can't stop it (new
        /// runners: on). Off runs it, and everything the host runs in
        /// Docker, Apple container and Lima sandboxes, as the sandbox's
        /// user, whose commands can stop it by accident and cut a turn
        /// short.
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        agent_as_root: Option<bool>,
        /// Retire this machine once its runner has run nothing for this
        /// long (20m, 1h; never: keep it): it drains, runs
        /// --retire-command, and leaves. For machines of a pool that grows
        /// and shrinks.
        #[arg(long, value_name = "DURATION|never", value_parser = retire_arg)]
        retire_when_idle: Option<RetireAfter>,
        /// What takes this machine away as it retires, run with `sh -c`:
        /// an autoscaling group's terminate call, or `systemctl poweroff`.
        #[arg(long, value_name = "COMMAND")]
        retire_command: Option<String>,
    },
    /// Run the host service in the foreground (what the service runs).
    Run,
    /// Show this runner: its host service's configuration and state, and
    /// what the control plane says of it (its id, sessions, whether it drains).
    Status,
    /// Check that everything this machine needs to run sessions as a
    /// runner is in place: its configuration and tokens, the connection,
    /// the driver and whether its sandboxes answer, the images, the disk
    /// and the service.
    Doctor,
    /// Print the host service's log.
    Logs {
        #[arg(short = 'n', long, default_value_t = 100)]
        lines: usize,
    },
    /// Pull the sandbox image, or build it with this runner's driver.
    #[command(subcommand)]
    Image(ImageCmd),
    /// Keep your Claude token on this runner rather than in fctl.
    #[command(subcommand)]
    Claude(super::host_claude::HostClaudeCmd),
    /// Restart the host service (sessions keep running).
    Restart,
    /// Whether the host updates fctl, and so its agents and sandbox image,
    /// to the version the control plane runs, by itself: on unless turned
    /// off.
    #[command(
        after_help = "Examples:\n  fctl host auto-update       # whether it does\n  fctl host auto-update off   # update with `fctl self update` instead\n  fctl host auto-update on"
    )]
    AutoUpdate {
        /// on or off. Without it, shows which it is. Changing it restarts
        /// the host service (sessions keep running).
        #[arg(value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        on: Option<bool>,
    },
    /// Drain this runner: it takes no new sessions; the ones here carry on,
    /// and a message resumes them here.
    #[command(after_help = "Examples:
  fctl host drain                      # no new sessions; the ones here carry on
  fctl host drain --wait               # until none runs here
  fctl host drain --timeout 60s        # on Spot's two-minute notice: stop what still runs after a minute")]
    Drain {
        /// Wait until no session runs here.
        #[arg(long)]
        wait: bool,
        /// Wait at most this long (90s, 10m, 1h), then stop the sessions
        /// that still run: their agents stop and push their work, and a
        /// message resumes each on another runner. Stopping takes up to a
        /// minute more. Implies --wait.
        #[arg(long, value_name = "DURATION", value_parser = timeout_arg)]
        timeout: Option<std::time::Duration>,
    },
    /// Undrain this runner: it takes new sessions again.
    Undrain,
    /// Stop and remove the host service, so this machine runs no more
    /// sessions (sandboxes are kept). When sessions run here, it says what
    /// happens to them and asks first.
    Uninstall {
        /// Drain this runner first, and uninstall once no session runs here.
        #[arg(long, conflicts_with = "force")]
        wait: bool,
        /// Stop the sessions that run here now, then uninstall. Their agents
        /// stop and push their work, and a message resumes each session on
        /// another runner.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub enum ImageCmd {
    /// Pull this runner's sandbox image now (again: for a newer build of it).
    Pull,
    /// Build the default image here instead of pulling it (or the Lima base VM).
    Build {
        /// apple, docker or lima (default: this runner's driver).
        #[arg(long)]
        driver: Option<String>,
        /// Image tag (default: this runner's image, so that sessions use the build).
        #[arg(long)]
        tag: Option<String>,
        /// A checkout of hyperstructure/sandbox to build from (default: its
        /// main branch, downloaded from GitHub).
        #[arg(long, value_name = "DIR")]
        source: Option<std::path::PathBuf>,
    },
}

pub async fn run(server: Option<&str>, cmd: HostCmd, json: bool) -> Result<()> {
    match cmd {
        HostCmd::Run => {
            let cfg = HostConfig::load()?;
            // A machine that retired waits to be taken away, whatever runs
            // the host again.
            if let Some(at) = fctl_host::retire::retired(&cfg.data_dir()) {
                eprintln!(
                    "This machine retired at {at} (retire_idle_minutes): the host runs nothing here until `fctl host install` makes it a runner again."
                );
                // Until stopped: SIGTERM too, as a container's PID 1 is.
                fctl_host::daemon::signalled().await;
                return Ok(());
            }
            let token = HostConfig::load_token()?;
            // The service manager may have started it without the group
            // that lets it use Docker, when its user joined that later.
            if uses_docker(&cfg.driver) {
                docker_access::regroup();
            }
            fctl_host::Daemon::new(cfg, token).await?.run().await
        }
        HostCmd::Install {
            name,
            driver,
            capacity,
            cpus,
            memory,
            max_cpus,
            max_memory,
            image,
            allow_images,
            allow_builds,
            workspace,
            personal,
            token,
            enroll,
            no_service,
            build_image,
            auto_update,
            agent_as_root,
            retire_when_idle,
            retire_command,
        } => {
            const SIGN_IN: &str = "Sign in first (`fctl login`), or pass --token or --enroll";
            // Docker that refuses this user stops it before anything else.
            if uses_docker(&driver) {
                docker_group()?;
            }
            let server = match (&enroll, config::server(server)) {
                (_, Ok(server)) => server,
                (Some(_), Err(_)) => bail!(
                    "Which fctl instance is the enrollment token for? Pass --server https://app.fctl.sh, or set FCTL_SERVER."
                ),
                (None, Err(e)) => return Err(e),
            };
            // The runner this machine has now, for the instance it runs for.
            let before = HostConfig::load().ok().map(|c| c.server);
            let mut cfg = HostConfig::load().unwrap_or_default();
            cfg.server = server.clone();
            if let Some(n) = name {
                cfg.name = n;
            }
            cfg.capacity = capacity.max(1);
            cfg.cpus = cpus.unwrap_or(cfg.cpus);
            cfg.memory_mb = memory.unwrap_or(cfg.memory_mb);
            cfg.max_cpus = max_cpus.or(cfg.max_cpus);
            cfg.max_memory_mb = max_memory.or(cfg.max_memory_mb);
            if let Some(i) = image {
                cfg.image = (!means_default_image(&i)).then_some(i);
            }
            if !allow_images.is_empty() {
                let list: Vec<String> = allow_images
                    .iter()
                    .map(|i| i.trim().to_owned())
                    .filter(|i| !i.is_empty())
                    .collect();
                cfg.allowed_images = (!list.iter().any(|i| i == "*")).then_some(list);
            }
            if !allow_builds.is_empty() {
                let list: Vec<String> = allow_builds
                    .iter()
                    .map(|r| r.trim().to_owned())
                    .filter(|r| !r.is_empty())
                    .collect();
                cfg.allowed_builds = (!list.is_empty()).then_some(list);
            }
            if let Some(on) = auto_update {
                cfg.auto_update = on;
            }
            if let Some(on) = agent_as_root {
                cfg.agent_as_root = on;
            }
            if let Some(RetireAfter(minutes)) = retire_when_idle {
                cfg.retire_idle_minutes = minutes;
            }
            if let Some(c) = retire_command {
                cfg.retire_command = Some(c.trim().to_owned()).filter(|c| !c.is_empty());
            }
            if let Some(problem) = cfg.image_problems().first() {
                bail!("{problem}");
            }
            // What the enrollment token enrolls in, or why it can't, before
            // anything else: that fails fastest.
            let enrolling = match enroll {
                Some(secret) => Some(Enrolling::ask(&server, secret, before.as_deref()).await?),
                None => None,
            };
            // Registering a runner leaves something behind, so it waits until
            // everything that can fail on this machine has passed.
            let api = if token.is_some() || enrolling.is_some() {
                None
            } else {
                Some(config::api(Some(&server)).context(SIGN_IN)?)
            };
            // `auto` picks like the daemon does, except that it never settles for
            // the process driver; neither picks Firecracker, which a host's
            // owner chooses.
            let driver = if driver == "auto" {
                let infos = Driver::probe_all(&cfg).await;
                match infos
                    .iter()
                    .find(|i| i.available && i.name != "process" && i.name != "firecracker")
                {
                    Some(info) => info.name.clone(),
                    None => bail!("{}", no_runtime(&infos, std::env::consts::OS)),
                }
            } else {
                driver
            };
            let d = Driver::select(&driver, &cfg).await?;
            if build_image {
                fctl_host::image::build(&d, &cfg.image(), &cfg.download_base, None).await?;
            }
            let info = d.probe().await;
            if !info.available {
                bail!(
                    "The {} driver is not ready: {}. Nothing was registered; run this again once it is.",
                    info.name,
                    info.detail.unwrap_or_default()
                );
            }
            println!(
                "{} Driver: {} ({})",
                style::accent("✓"),
                info.name,
                info.detail.unwrap_or_default()
            );
            println!(
                "{} Sandboxes: {}",
                style::accent("✓"),
                size::limits(&cfg.sandbox_limits(d.resources().await))
            );
            // The service would fail to install after the runner exists.
            if !no_service {
                service::preflight()?;
            }
            let token = match (token, &enrolling) {
                (Some(t), _) => t,
                (None, Some(e)) => e.finish(&cfg, d.name()).await?,
                (None, None) => {
                    let workspace = workspace.or_else(|| config::CliConfig::load().workspace);
                    let reg = api
                        .context(SIGN_IN)?
                        .create_runner(&cfg.name, workspace.as_deref(), personal.then_some(true))
                        .await?;
                    println!(
                        "{} Registered runner {} ({}){}",
                        style::accent("✓"),
                        style::bold(&reg.runner.name),
                        reg.runner.id,
                        match reg.runner.workspaces.as_slice() {
                            [] => " for no workspace yet".to_owned(),
                            ws => format!(
                                " for {}",
                                ws.iter()
                                    .map(|w| if reg.runner.personal.contains(&w.id) {
                                        format!("your sessions in {}", w.name)
                                    } else {
                                        format!("{}'s sessions", w.name)
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                        }
                    );
                    reg.token
                }
            };
            // The service starts with the driver checked here instead of picking
            // again, perhaps before a runtime is up.
            cfg.driver = d.name().into();
            cfg.save()?;
            HostConfig::save_token(&token)?;
            // Installed again, it is a runner again.
            let _ = std::fs::remove_file(fctl_host::retire::retired_marker(&cfg.data_dir()));
            println!(
                "{} Wrote {}",
                style::accent("✓"),
                HostConfig::path().display()
            );
            if !build_image && d.uses_images() {
                println!(
                    "{}",
                    style::dim(&format!(
                        "Sessions start from {}; the host pulls it when it starts.",
                        cfg.image()
                    ))
                );
                let images = cfg.images(d.builds_images());
                println!(
                    "{}",
                    style::dim(&format!(
                        "It {} and {}.",
                        image::describe_allowed(images.allowed.as_deref()),
                        image::describe_builds(images.builds.as_deref())
                    ))
                );
            } else if !build_image && d.needs_linux_agent() {
                println!(
                    "{}",
                    style::dim("Prepare the base VM once with `fctl host image build`.")
                );
            }
            println!("{}", style::dim(&updates_sentence(&cfg)));
            if let Some(retires) = retire_sentence(&cfg) {
                println!("{}", style::dim(&retires));
            }
            // The process driver has no sandboxes, and runs fctl-agent as
            // this machine's user either way.
            if !cfg.agent_as_root && d.needs_linux_agent() {
                println!("{}", style::dim(AGENT_AS_USER));
            }
            if !no_service {
                let exe = std::env::current_exe()?;
                let what = service::install(&exe)?;
                println!("{} Service: {what}", style::accent("✓"));
                if d.name() == "docker"
                    && let Some(why) = service_without_docker().await
                {
                    println!("{} {why}", style::amber("!"));
                }
                println!(
                    "{}",
                    style::dim(&format!("Logs: {}", service::log_location()))
                );
            }
            // A workspace's runner keeps nobody's Claude token.
            if enrolling.is_none() && matches!(claude_token::load(&cfg.claude_token()), Ok(None)) {
                println!(
                    "{}",
                    style::dim(
                        "To keep your Claude token here rather than in fctl: `claude setup-token`, then `fctl host claude set-token`."
                    )
                );
            }
            if cfg!(target_os = "macos") {
                println!();
                println!("{}", style::bold("Keep the Mac working 24/7:"));
                println!("  sudo pmset -a sleep 0 disksleep 0 standby 0 autorestart 1 womp 1");
                println!("  System Settings → Users & Groups → automatically log in as this user");
            }
            Ok(())
        }
        HostCmd::Status => {
            let cfg = HostConfig::load()?;
            // As the host service does, so that Docker answers this as it
            // answers the service.
            if uses_docker(&cfg.driver) {
                docker_access::regroup();
            }
            let (installed, running) = service::status();
            let driver = Driver::select(&cfg.driver, &cfg).await.ok();
            let (sandbox, (spot, free)) = match &driver {
                Some(d) => (
                    Some(cfg.sandbox_limits(d.resources().await)),
                    disk_here(&cfg, d).await,
                ),
                None => (None, (None, None)),
            };
            let min_free = cfg.min_free_disk_mb();
            let cached = caches::load(&caches::path(&cfg.data_dir()));
            // What the control plane says about this runner and its sessions.
            let runner = match runner_api() {
                Ok(api) => api.this_runner().await.map_err(anyhow::Error::from),
                Err(e) => Err(e),
            };
            // Whether its driver is ready: as the host service last said,
            // while it is connected, else as it answers here.
            let said = runner.as_ref().ok().and_then(|d| {
                let r = &d.runner;
                let name = r.driver.as_deref()?;
                (r.status == RunnerStatus::Online)
                    .then(|| r.drivers.iter().find(|i| i.name == name).cloned())
                    .flatten()
            });
            let driver_info = match (said, &driver) {
                (Some(info), _) => Some(info),
                (None, Some(d)) => Some(d.probe().await),
                (None, None) => None,
            };
            if json {
                let disk = serde_json::json!({
                    "at": spot.as_ref().map(DiskSpot::to_string),
                    "free_mb": free,
                    "min_free_mb": min_free,
                    "low": disk::low(free, min_free),
                });
                let caches = serde_json::json!({
                    "max_mb": cfg.max_cache_mb(),
                    "keep_days": (cfg.keep_caches_days > 0).then_some(cfg.keep_caches_days),
                    "projects": cached.projects,
                });
                // Whether it keeps a Claude token, never the token.
                let kept = claude_token::load(&cfg.claude_token()).ok().flatten();
                let claude = serde_json::json!({
                    "file": cfg.claude_token().display().to_string(),
                    "kept": kept.is_some(),
                    "saved_at": kept.map(|k| k.saved_at),
                });
                let updates = Updates::of(&cfg).json();
                return style::print_json(
                    &serde_json::json!({ "config": cfg, "image": cfg.image(), "sandbox": sandbox, "disk": disk, "caches": caches, "updates": updates, "claude_token": claude, "driver": driver_info, "service_installed": installed, "service_running": running, "runner": runner.as_ref().ok() }),
                );
            }
            println!("{} {}", style::bold(&cfg.name), style::dim(&cfg.server));
            if let Ok(d) = &runner {
                println!("id:       {}", d.runner.id);
            }
            match &driver_info {
                Some(info) if !info.available => println!(
                    "driver:   {}, {}",
                    cfg.driver,
                    style::amber(&format!(
                        "not ready: {}: it takes no sessions until it is",
                        info.detail.as_deref().unwrap_or("not available")
                    ))
                ),
                _ => println!("driver:   {}", cfg.driver),
            }
            println!("capacity: {} sessions", cfg.capacity);
            if let Some(l) = &sandbox {
                println!("sandbox:  {}", size::limits(l));
            }
            if let Some(spot) = &spot {
                println!("disk:     {}", disk_line(spot, free, min_free));
            }
            if let Some(d) = &driver {
                println!("caches:   {}", caches_line(&cfg, d, &cached));
            }
            println!("image:    {}", cfg.image());
            println!("updates:  {}", updates_line(&cfg));
            if let Some(retires) = retire_sentence(&cfg) {
                println!("retires:  {}", style::dim(&retires));
            }
            println!("agent:    {}", agent_line(&cfg));
            println!("claude:   {}", claude_line(&cfg));
            if let Some(d) = &driver {
                let images = cfg.images(d.builds_images());
                println!(
                    "images:   {}; {}",
                    image::describe_allowed(images.allowed.as_deref()),
                    image::describe_builds(images.builds.as_deref())
                );
            }
            println!(
                "service:  {}",
                match (installed, running) {
                    (true, true) => style::accent("running"),
                    (true, false) => style::amber("installed, not running"),
                    _ => style::dim("not installed"),
                }
            );
            match runner {
                Ok(d) => {
                    let state = match d.runner.status {
                        RunnerStatus::Online => style::accent("online"),
                        RunnerStatus::Offline => style::dim("offline"),
                        RunnerStatus::Disabled => style::red("disabled"),
                    };
                    match (&d.runner.workspace, d.runner.pending_until) {
                        (Some(w), Some(until)) => println!(
                            "runner:   {state}, {}",
                            style::amber(&format!(
                                "waiting for {} to approve it: it takes sessions once approved (`fctl runners approve {}`), and is removed {} unless it is",
                                match &d.runner.created_by {
                                    Some(u) => u.name.clone(),
                                    None => format!("an admin of {}", w.name),
                                },
                                d.runner.id,
                                style::until(until)
                            ))
                        ),
                        _ => match admin::drain_line(&d.runner) {
                            Some(drain) => println!("runner:   {state}, {}", style::amber(&drain)),
                            None => println!("runner:   {state}"),
                        },
                    }
                    if let Some(w) = &d.runner.workspace {
                        println!("workspace: {}", workspace_line(&w.name, &d.runner));
                    }
                    if !d.sessions.is_empty() {
                        println!("sessions:");
                        admin::print_sessions(&d.sessions);
                    }
                    print_autopilots(&d.autopilots, d.runner.drain.is_some());
                }
                Err(e) => println!(
                    "runner:   {}",
                    style::dim(&format!("could not ask {}: {e:#}", cfg.server))
                ),
            }
            Ok(())
        }
        HostCmd::Doctor => doctor().await,
        HostCmd::Logs { lines } => match service::log_tail(lines) {
            Ok(Some(tail)) => {
                for l in &tail {
                    println!("{l}");
                }
                Ok(())
            }
            // No log, or no journalctl to read one, and no host.toml: no host.
            _ if !HostConfig::path().exists() => bail!(NOT_A_RUNNER),
            Ok(None) => bail!("{}", service::why_no_log()),
            Err(e) => Err(e),
        },
        HostCmd::Image(ImageCmd::Pull) => {
            let cfg = HostConfig::load().unwrap_or_default();
            let d = Driver::select(&cfg.driver, &cfg).await?;
            if !d.uses_images() {
                bail!(
                    "The {} driver doesn't use images{}",
                    d.name(),
                    if d.needs_linux_agent() {
                        ": its sessions clone the base VM that `fctl host image build` prepares"
                    } else {
                        ""
                    }
                );
            }
            let image = cfg.image();
            let note = |line: String| {
                if !json {
                    println!("{line}");
                }
            };
            // The default image falls back to `latest` until its version's is published.
            let pulled = refresh::pull(&d, &cfg, !json, &note).await?;
            let removed = refresh::clean_up(&d, &cfg).await;
            let instead_of = (pulled.image != image).then_some(&image);
            if json {
                return style::print_json(&serde_json::json!({
                    "image": pulled.image,
                    "instead_of": instead_of,
                    "newer": pulled.newer.is_some(),
                    "removed": removed,
                }));
            }
            let newer = match &pulled.newer {
                Some(build) => format!(", a newer build{}", refresh::claude_code(build)),
                None => String::new(),
            };
            println!("{} Pulled {}{newer}", style::accent("✓"), pulled.image);
            if instead_of.is_some() {
                println!(
                    "  {}",
                    style::dim(&format!(
                        "Sessions start from it until {image} can be pulled: a release's image is published minutes after the release."
                    ))
                );
            }
            for name in &removed {
                let what = refresh::describe(name);
                println!("  {}", style::dim(&format!("Removed {what}")));
            }
            if let Some(name) = &pulled.replaced
                && !removed.contains(name)
            {
                println!(
                    "  {}",
                    style::dim(
                        "The build it replaced stays until no sandbox uses it; the host removes it then."
                    )
                );
            }
            Ok(())
        }
        HostCmd::Image(ImageCmd::Build {
            driver,
            tag,
            source,
        }) => {
            let cfg = HostConfig::load().unwrap_or_default();
            let name = driver.unwrap_or(cfg.driver.clone());
            let d = Driver::select(&name, &cfg).await?;
            let tag = tag.unwrap_or_else(|| cfg.image());
            fctl_host::image::build(&d, &tag, &cfg.download_base, source.as_deref()).await
        }
        HostCmd::Restart => {
            if !HostConfig::path().exists() {
                bail!(NOT_A_RUNNER);
            }
            service::restart()?;
            println!("Restarted. Sessions keep running in their sandboxes.");
            Ok(())
        }
        HostCmd::AutoUpdate { on } => auto_update(on, json).await,
        HostCmd::Drain { wait, timeout } => {
            let wait = wait || timeout.is_some();
            let api = runner_api()?;
            let mut d = api.drain_this_runner(true, false).await?;
            if !json {
                println!(
                    "{} Draining {}: no new sessions start here.",
                    style::accent("✓"),
                    style::bold(&d.runner.name)
                );
                println!(
                    "{}",
                    style::dim(&admin::still_running(d.sessions.len() as u32, "here"))
                );
                admin::print_sessions(&d.sessions);
            }
            if wait {
                d = wait_until_drained(&api, d, json, timeout).await?;
                if !d.sessions.is_empty() {
                    // Out of time: what still runs stops, and pushes.
                    if !json {
                        println!(
                            "{}",
                            style::amber(&format!(
                                "Out of time: stopping {}. {} {} and push{} {} work, and a message resumes {} on another runner.",
                                if d.sessions.len() == 1 {
                                    "the session that still runs".to_owned()
                                } else {
                                    format!("the {} sessions that still run", d.sessions.len())
                                },
                                if d.sessions.len() == 1 {
                                    "Its agent"
                                } else {
                                    "Their agents"
                                },
                                if d.sessions.len() == 1 {
                                    "stops"
                                } else {
                                    "stop"
                                },
                                if d.sessions.len() == 1 { "es" } else { "" },
                                if d.sessions.len() == 1 {
                                    "its"
                                } else {
                                    "their"
                                },
                                if d.sessions.len() == 1 { "it" } else { "each" },
                            ))
                        );
                    }
                    let stopping = stop_sessions(&api, json, STOP_AFTER_TIMEOUT).await?;
                    d = wait_until_stopped(&api, stopping, json, STOP_AFTER_TIMEOUT).await;
                }
                if !json {
                    print_autopilots(&d.autopilots, d.runner.drain.is_some());
                }
            } else if !json && !d.sessions.is_empty() {
                println!(
                    "{}",
                    style::dim(
                        "`fctl host drain --wait` waits until none runs here; `fctl host undrain` takes new sessions again."
                    )
                );
            }
            if json {
                return style::print_json(&d);
            }
            Ok(())
        }
        HostCmd::Undrain => {
            let d = runner_api()?.drain_this_runner(false, false).await?;
            if json {
                return style::print_json(&d);
            }
            println!(
                "{} {} takes new sessions again.",
                style::accent("✓"),
                style::bold(&d.runner.name)
            );
            Ok(())
        }
        HostCmd::Uninstall { wait, force } => uninstall(wait, force, json).await,
        HostCmd::Claude(cmd) => super::host_claude::run(cmd, json).await,
    }
}

/// `fctl host install --enroll`: the enrollment token, what it enrolls in,
/// and the runner this machine has there already, if any.
struct Enrolling {
    api: fctl_client::Api,
    info: EnrollmentInfo,
    /// The runner token this machine has, and its runner, when that runner
    /// belongs to the same workspace: installing again keeps it.
    kept: Option<(String, Runner)>,
}

impl Enrolling {
    /// Asks the instance at `server` what the enrollment token `secret`
    /// enrolls in, and whether this machine's runner (`before`: the
    /// instance it runs for) is one of that workspace's already.
    async fn ask(server: &str, secret: String, before: Option<&str>) -> Result<Self> {
        let secret = secret.trim().to_owned();
        if !fctl_domain::tokens::looks_like(&secret, fctl_domain::tokens::SecretKind::Enrollment) {
            bail!(
                "That isn't an enrollment token: they start with fctl_et_. A workspace's admins make them on its Runners tab, or with `fctl runners enrollment-tokens create`."
            );
        }
        let api = fctl_client::Api::new(server, Some(secret));
        let info = api.enrollment().await.map_err(|e| match e.api() {
            Some(a) if a.code == fctl_proto::ErrorCode::NotFound => anyhow::anyhow!(
                "{server} doesn't take enrollment tokens: it runs an older version of fctl. Register this machine signed in instead (`fctl login`, then `fctl host install`)."
            ),
            Some(a) => anyhow::anyhow!("{}", a.message),
            None => anyhow::anyhow!("Could not reach {server}: {e}"),
        })?;
        let same_server =
            before.is_some_and(|b| config::normalize_server(b) == config::normalize_server(server));
        let kept = match HostConfig::load_token() {
            Ok(t) if same_server => fctl_client::Api::new(server, Some(t.clone()))
                .this_runner()
                .await
                .ok()
                .filter(|d| {
                    d.runner
                        .workspace
                        .as_ref()
                        .is_some_and(|w| w.id == info.workspace.id)
                        && d.runner.created_by.as_ref().map(|u| &u.id)
                            == info.owner.as_ref().map(|u| &u.id)
                })
                .map(|d| (t, d.runner)),
            _ => None,
        };
        let how = match (info.approval, &info.owner) {
            (false, _) => String::new(),
            (true, None) => ", once one of its admins approves it".to_owned(),
            (true, Some(u)) => format!(", once {} approves it", u.name),
        };
        match &kept {
            Some((_, r)) => println!(
                "{} {} is a runner of {} already: installing again keeps it",
                style::accent("✓"),
                style::bold(&r.name),
                info.workspace.name
            ),
            None => println!(
                "{} Enrolls in {} with “{}”: the runner runs {} there{how}",
                style::accent("✓"),
                style::bold(&info.workspace.name),
                info.name,
                match &info.owner {
                    Some(u) => format!("only {}'s sessions", u.name),
                    None => "everyone's sessions".to_owned(),
                }
            ),
        }
        Ok(Self { api, info, kept })
    }

    /// The runner token to install: the one this machine has, or a new
    /// runner's, which it enrolls now.
    async fn finish(&self, cfg: &HostConfig, driver: &str) -> Result<String> {
        if let Some((token, _)) = &self.kept {
            return Ok(token.clone());
        }
        let facts = RunnerFacts {
            version: Some(update::VERSION.to_owned()),
            hostname: Some(fctl_host::config::hostname()),
            os: Some(fctl_host::sysinfo::os().to_owned()),
            arch: Some(fctl_host::sysinfo::arch().to_owned()),
            driver: Some(driver.to_owned()),
            cpus: Some(fctl_host::sysinfo::cpus()),
            memory_mb: Some(fctl_host::sysinfo::memory_mb()),
            capacity: Some(cfg.capacity),
        };
        let reg = self
            .api
            .enroll(&EnrollRunnerRequest {
                name: cfg.name.clone(),
                facts: Some(facts),
            })
            .await
            .map_err(|e| match e.api() {
                Some(a) => anyhow::anyhow!("{}", a.message),
                None => anyhow::anyhow!("Could not enroll this machine: {e}"),
            })?;
        let r = &reg.runner;
        let workspace = &self.info.workspace.name;
        println!(
            "{} Enrolled runner {} ({}) in {workspace}",
            style::accent("✓"),
            style::bold(&r.name),
            r.id
        );
        match (r.pending_until, &self.info.owner) {
            (Some(until), None) => println!(
                "{}",
                style::amber(&format!(
                    "It waits for an admin of {workspace} to approve it, and takes sessions as soon as one does: they see it on {workspace}'s Runners tab, or run `fctl runners approve {}`. It is removed {} unless one does.",
                    r.id,
                    style::until(until)
                ))
            ),
            (Some(until), Some(u)) => println!(
                "{}",
                style::amber(&format!(
                    "It waits for {} to approve it, and takes their sessions as soon as they do: they see it on {workspace}'s Runners tab, or run `fctl runners approve {}`. It is removed {} unless they do.",
                    u.name,
                    r.id,
                    style::until(until)
                ))
            ),
            (None, None) => println!(
                "{}",
                style::dim(&format!("It runs everyone's sessions in {workspace}."))
            ),
            (None, Some(u)) => println!(
                "{}",
                style::dim(&format!(
                    "It runs only {}'s sessions in {workspace}: it is theirs.",
                    u.name
                ))
            ),
        }
        let gone = match r.remove_offline_after_minutes {
            Some(h) => format!(
                "It is removed once it has been offline for {}, and when `fctl host uninstall` uninstalls it.",
                fctl_domain::enrollment::minutes(h)
            ),
            None => "`fctl host uninstall` removes it from the workspace.".to_owned(),
        };
        println!("{}", style::dim(&gone));
        Ok(reg.token)
    }
}

/// `fctl host uninstall`: sessions that run here finish first (`--wait`) or
/// stop properly (`--force`, or yes when asked), so that no agent keeps
/// working where nobody sees it.
async fn uninstall(wait: bool, force: bool, json: bool) -> Result<()> {
    let asked = match runner_api() {
        Ok(api) => match api.this_runner().await {
            Ok(d) => Ok((api, d)),
            Err(e) => Err(anyhow::Error::from(e)),
        },
        Err(e) => Err(e),
    };
    let mut drained = false;
    // A runner that enrolled in a workspace goes with its host.
    let enrolled = match &asked {
        Ok((api, d)) => d
            .runner
            .workspace
            .as_ref()
            .map(|w| (api.clone(), d.runner.clone(), w.name.clone())),
        Err(_) => None,
    };
    match asked {
        Ok((_, d)) if d.sessions.is_empty() => {}
        Ok((api, d)) => {
            let n = d.sessions.len();
            let online = d.runner.status == RunnerStatus::Online;
            if !json {
                println!(
                    "{} on this runner:",
                    match n {
                        1 => "1 session runs".to_owned(),
                        n => format!("{n} sessions run"),
                    }
                );
                admin::print_sessions(&d.sessions);
                println!(
                    "{}",
                    style::dim(if wait {
                        "Draining this runner: no new sessions start here, and it uninstalls once none runs."
                    } else if n == 1 {
                        "Uninstalling now stops it: its agent stops and pushes what it committed, and a message resumes the session on another runner, in a new sandbox without its conversation."
                    } else {
                        "Uninstalling now stops them: their agents stop and push what they committed, and a message resumes each on another runner, in a new sandbox without its conversation."
                    })
                );
            }
            if wait {
                if !json && !online {
                    println!(
                        "{}",
                        style::dim(
                            "The host isn't connected, so its sessions can't finish here: the control plane stops them within ten minutes."
                        )
                    );
                }
                let d = api.drain_this_runner(true, false).await?;
                wait_until_drained(&api, d, json, None).await?;
            } else {
                let interactive = !json && std::io::stdin().is_terminal();
                let stop =
                    force || (interactive && confirm("Stop them and uninstall now? [y/N] ")?);
                if !stop {
                    let (them, sessions) = if n == 1 {
                        ("it", "1 session runs".to_owned())
                    } else {
                        ("them", format!("{n} sessions run"))
                    };
                    if interactive {
                        println!(
                            "Nothing changed. `fctl host uninstall --wait` lets {them} finish first."
                        );
                        return Ok(());
                    }
                    bail!(
                        "{sessions} on this runner. `fctl host uninstall --wait` lets {them} finish first; `--force` stops {them} now."
                    );
                }
                let d = api.drain_this_runner(true, true).await?;
                if online {
                    wait_until_stopped(&api, d, json, std::time::Duration::from_secs(120)).await;
                } else {
                    // Nothing reaches its host: stop the sandboxes here.
                    if !json {
                        println!(
                            "{}",
                            style::dim(
                                "The host isn't connected, so this stops the sandboxes here directly: their agents don't get to push what they haven't yet."
                            )
                        );
                    }
                    stop_sandboxes_here(json).await;
                }
            }
            drained = true;
        }
        Err(e) => {
            // The control plane can't say what runs here: look at the sandboxes.
            let running = running_sandboxes().await;
            if running > 0 && !force {
                let sandboxes = if running == 1 {
                    "a session's sandbox still runs".to_owned()
                } else {
                    format!("{running} sessions' sandboxes still run")
                };
                bail!(
                    "Could not ask the control plane which sessions run here ({e:#}), and {sandboxes} on this machine: their agents would keep working without reaching fctl. `fctl host uninstall --force` stops them and uninstalls."
                );
            }
            if running > 0 {
                stop_sandboxes_here(json).await;
            }
        }
    }
    service::uninstall()?;
    let removed = match &enrolled {
        Some((api, _, _)) => Some(api.remove_this_runner().await),
        None => None,
    };
    if let Some(Ok(_)) = &removed
        && let Err(e) = HostConfig::remove_token()
    {
        eprintln!(
            "{}",
            style::red(&format!("removing the runner token: {e:#}"))
        );
    }
    if json {
        return style::print_json(&serde_json::json!({
            "uninstalled": true,
            "drained": drained,
            "runner_removed": removed.as_ref().map(Result::is_ok),
        }));
    }
    println!("Removed the service. Configuration and sandboxes are kept.");
    if let Some((_, r, workspace)) = &enrolled {
        match &removed {
            Some(Ok(_)) => println!(
                "{} Removed runner {} from {workspace}: it enrolled there, and its token no longer works. `fctl host install --enroll` enrolls this machine again.",
                style::accent("✓"),
                style::bold(&r.name)
            ),
            Some(Err(e)) => println!(
                "{}",
                style::amber(&format!(
                    "Could not remove runner {} from {workspace} ({}): {} remove{} it from its Runners tab{}.",
                    r.name,
                    e.api().map_or_else(|| e.to_string(), |a| a.message.clone()),
                    match &r.created_by {
                        Some(u) => u.name.clone(),
                        None => "its admins".to_owned(),
                    },
                    if r.created_by.is_some() { "s" } else { "" },
                    match r.remove_offline_after_minutes {
                        Some(h) => format!(
                            ", or fctl does once it has been offline for {}",
                            fctl_domain::enrollment::minutes(h)
                        ),
                        None => String::new(),
                    }
                ))
            ),
            None => {}
        }
        return Ok(());
    }
    if drained {
        println!(
            "{}",
            style::dim(
                "The runner keeps draining: once the host runs again, `fctl host undrain` lets it take new sessions."
            )
        );
    }
    Ok(())
}

/// Why `fctl host install --driver auto` has no driver to pick, from what
/// the drivers' probes (`infos`) say on `os`: what each one that runs there
/// lacks, Firecracker's too, which it uses only when asked.
fn no_runtime(infos: &[DriverInfo], os: &str) -> String {
    let runs_here = |i: &&DriverInfo| match i.name.as_str() {
        "process" => false,
        "apple" => os == "macos",
        "firecracker" => os == "linux",
        _ => true,
    };
    let firecracker = infos
        .iter()
        .any(|i| i.name == "firecracker" && i.available && os == "linux");
    let mut why = String::from(if firecracker {
        "Only Firecracker is ready on this machine, and fctl uses it only when asked:"
    } else {
        "No VM or container runtime is ready on this machine:"
    });
    for info in infos.iter().filter(runs_here) {
        let detail = info.detail.as_deref().unwrap_or_default();
        why.push_str(&format!("\n  {}: {detail}", info.name));
    }
    why.push_str(if firecracker {
        "\nPass --driver firecracker to run each session in a microVM of its own, or install another."
    } else if os == "linux" {
        "\nInstall one (fctl uses Firecracker only with --driver firecracker), or pass --driver process to run agents without isolation."
    } else {
        "\nInstall one, or pass --driver process to run agents without isolation."
    });
    why
}

/// Whether a host on `driver` would use Docker, on Linux, where its socket
/// lets in only root and the members of its group.
fn uses_docker(driver: &str) -> bool {
    cfg!(target_os = "linux")
        && match driver {
            "docker" => true,
            "auto" => fctl_host::driver::process::which("docker").is_some(),
            _ => false,
        }
}

/// Before `fctl host install` registers anything: when Docker's socket
/// refuses this user for want of its group, offers to add them, and then
/// starts this command again in the group, so that nobody has to log in
/// again; the service takes the group the same way as it starts. A user in
/// the group whose shell started before they joined it just starts again.
fn docker_group() -> Result<()> {
    docker_access::regroup();
    let Some(Problem::NotInGroup { user, group }) = docker_access::problem() else {
        return Ok(());
    };
    let join = format!("sudo usermod -aG {group} {user}");
    let again = "then run this again: fctl takes the group itself, with no need to log in again. Nothing was registered.";
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        bail!(
            "{user} isn't in the {group} group, so fctl can't use Docker. `{join}` adds them; {again}"
        );
    }
    println!("{user} isn't in the {group} group, so fctl can't use Docker.");
    if !confirm_yes(&format!("Add {user} to it now? This runs `{join}` [Y/n] "))? {
        bail!("`{join}` adds {user} to the {group} group; {again}");
    }
    let joined = std::process::Command::new("sudo")
        .args(["usermod", "-aG", &group, &user])
        .status()
        .context("running sudo")?;
    if !joined.success() {
        bail!("`{join}` failed. Nothing was registered.");
    }
    println!("{} Added {user} to the {group} group", style::accent("✓"));
    // Returns only when it can't start again in the group.
    docker_access::regroup();
    bail!(
        "{user} is in the {group} group now, but fctl can't take the group without `sg`: restart the machine, then run this again. Nothing was registered."
    )
}

/// Why the host service `fctl host install` just started can't use Docker,
/// when it can't: it takes the Docker socket's group as it starts, so this
/// waits a few seconds for that. `None` when it has the group, or when
/// that can't be told.
async fn service_without_docker() -> Option<String> {
    let mut lacks = None;
    for _ in 0..20 {
        let pid = tokio::task::spawn_blocking(service::main_pid)
            .await
            .ok()
            .flatten();
        lacks = match pid.map(docker_access::process_has_group) {
            Some(Some(true)) => return None,
            Some(None) => return None,
            Some(Some(false)) => pid,
            None => None,
        };
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    lacks?;
    Some(format!(
        "The host service can't use Docker: it started without the docker group, and couldn't take it. `{}` (or restarting the machine) gives it the group; then check with `fctl host status`.",
        docker_access::restart_manager()
    ))
}

/// Asks a yes-or-no question on the terminal, whose answer is yes unless
/// it says no. Ending the input (Ctrl-D) says no.
fn confirm_yes(question: &str) -> Result<bool> {
    use std::io::Write;
    print!("{question}");
    std::io::stdout().flush()?;
    let answer = yes_unless_no(&mut std::io::stdin().lock())?;
    if answer.is_none() {
        // What follows goes on a line of its own.
        println!();
    }
    Ok(answer == Some(true))
}

/// The answer to a question whose answer is yes unless it says no, from
/// the line `input` gives; `None` when it ends before a line.
fn yes_unless_no(input: &mut impl std::io::BufRead) -> Result<Option<bool>> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(!matches!(line.trim(), "n" | "N" | "no" | "No")))
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

/// Waits, for `limit` at most, until the sessions this runner was asked to
/// stop have stopped.
async fn wait_until_stopped(
    api: &fctl_client::Api,
    mut d: RunnerDetail,
    json: bool,
    limit: std::time::Duration,
) -> RunnerDetail {
    if !json {
        println!("{}", style::dim("Stopping…"));
    }
    let deadline = std::time::Instant::now() + limit;
    while !d.sessions.is_empty() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        // A poll that fails changes nothing: the next may go through.
        match api.this_runner().await {
            Ok(next) => d = next,
            Err(e) => poll_failed(json, &e),
        }
    }
    if !json {
        match d.sessions.len() {
            0 => println!("{} No sessions run here.", style::accent("✓")),
            1 => println!(
                "{}",
                style::amber(&format!(
                    "A session hadn't stopped after {}; going on anyway. Its agent stops on its own once it is idle.",
                    duration_words(limit)
                ))
            ),
            n => println!(
                "{}",
                style::amber(&format!(
                    "{n} sessions hadn't stopped after {}; going on anyway. Their agents stop on their own once they are idle.",
                    duration_words(limit)
                ))
            ),
        }
    }
    d
}

/// The sandboxes of sessions that run on this machine, by the host's driver.
async fn running_sandboxes() -> usize {
    let Ok(cfg) = HostConfig::load() else {
        return 0;
    };
    let Ok(d) = Driver::select(&cfg.driver, &cfg).await else {
        return 0;
    };
    d.list()
        .await
        .map(|l| {
            l.iter()
                .filter(|s| s.running && fctl_host::driver::session_of(&s.name).is_some())
                .count()
        })
        .unwrap_or(0)
}

/// Stops the sandboxes of sessions on this machine with the driver, when
/// the host can't stop them itself. Their agents don't get to push.
async fn stop_sandboxes_here(json: bool) {
    let Ok(cfg) = HostConfig::load() else {
        return;
    };
    let Ok(d) = Driver::select(&cfg.driver, &cfg).await else {
        return;
    };
    let Ok(list) = d.list().await else { return };
    for sb in list
        .iter()
        .filter(|s| s.running && fctl_host::driver::session_of(&s.name).is_some())
    {
        if !json {
            println!("{}", style::dim(&format!("Stopping {}…", sb.name)));
        }
        if let Err(e) = d.stop(&d.handle(&sb.name)).await {
            eprintln!("{}", style::red(&format!("stopping {}: {e:#}", sb.name)));
        }
    }
}

/// Whether the host updates itself, and why not when it doesn't.
struct Updates {
    /// `auto_update` in host.toml.
    on: bool,
    /// Why fctl can't update itself here, when it can't (Homebrew installed
    /// it, its directory can't be written to).
    blocked: Option<String>,
}

impl Updates {
    fn of(cfg: &HostConfig) -> Self {
        let blocked = match update::Install::here() {
            Ok(install) => install.blocked().map(|b| b.to_string()),
            Err(e) => Some(format!("{e:#}")),
        };
        Self {
            on: cfg.auto_update,
            blocked,
        }
    }

    /// Whether the host updates itself: on, and it can.
    fn itself(&self) -> bool {
        self.on && self.blocked.is_none()
    }

    /// Why it doesn't, when it doesn't.
    fn why_not(&self) -> Option<String> {
        match (&self.blocked, self.on) {
            (_, false) => Some("off (auto_update = false in host.toml)".into()),
            (Some(why), true) => Some(why.clone()),
            (None, true) => None,
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "auto_update": self.on,
            "updates_itself": self.itself(),
            "blocked": self.blocked,
        })
    }
}

/// What `fctl host install` says of a host whose fctl-agent runs as the
/// sandbox's user.
const AGENT_AS_USER: &str = "Nothing runs as root in its sandboxes but Firecracker's VMs' init: fctl-agent runs as the sandbox's user, whose commands can stop it by accident and cut the agent's turn short (`--agent-as-root on` runs it as root).";

/// `fctl host status`'s line about who fctl-agent runs as.
fn agent_line(cfg: &HostConfig) -> String {
    if cfg.driver == "process" {
        "fctl-agent runs as this machine's user (the process driver has no sandboxes)".into()
    } else if cfg.agent_as_root {
        "fctl-agent runs as root in sandboxes, out of the agent's reach".into()
    } else {
        style::amber(
            "fctl-agent runs as the sandbox's user (agent_as_root = false): the agent's commands can stop it",
        )
    }
}

/// `fctl host status`'s line about updates.
fn updates_line(cfg: &HostConfig) -> String {
    let u = Updates::of(cfg);
    match (u.blocked, u.on) {
        (None, true) => "automatic, to the version the control plane runs (`fctl host auto-update off` turns that off)".into(),
        (_, false) => style::dim("off (auto_update = false): `fctl self update` updates it"),
        (Some(why), true) => style::amber(&format!("none by itself: {why}")),
    }
}

/// What `fctl host install` says about updates.
fn updates_sentence(cfg: &HostConfig) -> String {
    let u = Updates::of(cfg);
    match (u.blocked, u.on) {
        (None, true) => "It updates fctl to the version the control plane runs whenever that is newer (`--auto-update off` turns that off).".into(),
        (_, false) => "It doesn't update fctl by itself: `fctl self update` updates it.".into(),
        (Some(why), true) => format!("It doesn't update fctl by itself: {why}."),
    }
}

/// `fctl host auto-update [on|off]`: shows or sets whether the host
/// updates itself. Setting it restarts the host service, which reads it
/// as it starts.
async fn auto_update(on: Option<bool>, json: bool) -> Result<()> {
    let cfg = HostConfig::load()?;
    let changed = on.is_some_and(|on| on != cfg.auto_update);
    let cfg = match on {
        Some(on) if changed => {
            HostConfig::save_auto_update(on)?;
            HostConfig::load()?
        }
        _ => cfg,
    };
    let (installed, running) = service::status();
    let restarted = changed && installed && running && service::restart().is_ok();
    // What the control plane runs, to say whether this host is behind.
    let theirs = fctl_client::Api::new(&cfg.server, None)
        .instance()
        .await
        .ok()
        .map(|i| i.version);
    let behind = theirs
        .as_deref()
        .and_then(|v| fctl_domain::updates::update_to(update::VERSION, v));
    let u = Updates::of(&cfg);
    if json {
        let mut out = u.json();
        out["version"] = update::VERSION.into();
        out["control_plane_version"] = theirs.into();
        out["changed"] = changed.into();
        out["restarted"] = restarted.into();
        return style::print_json(&out);
    }
    match (&u.blocked, u.on) {
        (None, true) => println!(
            "{} The host updates fctl to the version the control plane runs, whenever that is newer.",
            style::accent("✓")
        ),
        (_, false) => {
            println!("The host doesn't update fctl by itself: `fctl self update` updates it.")
        }
        (Some(why), true) => println!(
            "{}",
            style::amber(&format!("The host doesn't update fctl by itself: {why}."))
        ),
    }
    if changed {
        if restarted {
            println!("Restarted the host service; its sessions keep running.");
        } else if installed {
            println!("Restart the host service for it to take effect: fctl host restart");
        } else {
            println!("It takes effect when the host next starts.");
        }
    }
    if let Some(v) = behind {
        if u.itself() {
            println!(
                "{}",
                style::dim(&format!(
                    "The control plane runs fctl {v}, this host {}: it updates once {v} can be downloaded and no session is being started or stopped here.",
                    update::VERSION
                ))
            );
        } else {
            println!(
                "{}",
                style::dim(&format!(
                    "The control plane runs fctl {v}, this host {}.",
                    update::VERSION
                ))
            );
        }
    }
    Ok(())
}

/// `fctl host status`'s line about the workspace an enrolled runner
/// belongs to: `Acme's, enrolled with “Build fleet”; removed once offline
/// for an hour, and when the host uninstalls`, or, for a member's own,
/// `Acme, Ada's runner for their sessions, enrolled with …`.
fn workspace_line(workspace: &str, r: &Runner) -> String {
    let with = r
        .enrollment
        .as_ref()
        .and_then(|e| e.token_name.as_deref())
        .map(|t| format!(", enrolled with “{t}”"))
        .unwrap_or_default();
    let gone = match r.remove_offline_after_minutes {
        Some(h) => format!(
            "removed once offline for {}, and when the host uninstalls",
            fctl_domain::enrollment::minutes(h)
        ),
        None => "removed when the host uninstalls".to_owned(),
    };
    let whose = match &r.created_by {
        Some(u) => format!("{workspace}, {}'s runner for their sessions", u.name),
        None => format!("{workspace}'s"),
    };
    format!("{whose}{with}; {}", style::dim(&gone))
}

/// `fctl host status`'s line about the Claude token the host keeps.
fn claude_line(cfg: &HostConfig) -> String {
    match claude_token::load(&cfg.claude_token()) {
        Ok(Some(k)) => format!(
            "keeps your token ({}), saved {}",
            fctl_domain::claude::masked(&k.token),
            style::ago(k.saved_at)
        ),
        Ok(None) => style::dim("keeps no token (`fctl host claude set-token`)"),
        Err(e) => style::red(&format!("{e:#}")),
    }
}

/// The control plane, as this machine's runner (its token).
fn runner_api() -> Result<fctl_client::Api> {
    let cfg = HostConfig::load()?;
    let token = HostConfig::load_token()?;
    Ok(fctl_client::Api::new(&cfg.server, Some(token)))
}

/// This machine's runner, for commands about a runner that leave it out.
/// `api` is the instance they go to, which must be the one it runs for.
pub(crate) async fn this_runner(api: &fctl_client::Api) -> Result<Runner> {
    const NAME_IT: &str = "Which runner? This machine isn't one: give the runner's name or id (`fctl runners` lists them)";
    if !HostConfig::path().exists() {
        bail!(NAME_IT);
    }
    let cfg = HostConfig::load()?;
    let token = HostConfig::load_token()?;
    if config::normalize_server(&cfg.server) != config::normalize_server(api.base()) {
        bail!(
            "This machine is a runner of {}, not of {}: name the runner or give its id",
            cfg.server,
            api.base()
        );
    }
    // `main` prints an API error's message alone, which would drop a context.
    let d = fctl_client::Api::new(&cfg.server, Some(token))
        .this_runner()
        .await
        .map_err(|e| {
            let why = e.api().map_or_else(|| e.to_string(), |a| a.message.clone());
            anyhow::anyhow!(
                "Could not ask {} which runner this machine is: {why}",
                cfg.server
            )
        })?;
    Ok(d.runner)
}

/// The autopilots whose next wake resumes their own session here, where
/// its sandbox is, and what draining the runner (`draining`: it drains)
/// or shutting the machine down does to them. While it drains, the
/// control plane lists only those paused here mid-turn, which `fctl host
/// drain --wait` waits for as for any session.
fn print_autopilots(autopilots: &[RunnerAutopilot], draining: bool) {
    if autopilots.is_empty() {
        return;
    }
    println!("autopilots: their sessions' sandboxes are here, so they wake on this runner:");
    for a in autopilots {
        let s = &a.session;
        let wake = a
            .wake_at
            .map(|at| format!(", wakes {}", style::until(at)))
            .unwrap_or_default();
        println!(
            "  {} {}#{} {}{}",
            style::status(s.status),
            s.project_slug,
            s.number,
            style::dim(&style::plain(&s.title)),
            style::dim(&wake)
        );
    }
    println!("{}", style::dim(autopilots_note(draining)));
}

/// What [`print_autopilots`] says under its list.
fn autopilots_note(draining: bool) -> &'static str {
    if draining {
        "As the runner drains, only those paused here mid-turn are listed: each carries on here once its usage limit resets, and once that turn has ended, its next wake starts a fresh session on another runner, which takes over from its plan, log and files. Shutting the machine down cuts off one at work, which fctl stops 10 minutes later."
    } else {
        "Draining the runner moves each one off once its turn has ended: its next wake starts a fresh session on another runner, which takes over from its plan, log and files. So does its next wake while the machine is away, when another runner would run it. Shutting the machine down cuts off one at work, which fctl stops 10 minutes later."
    }
}

/// Says that reading the runner failed, and that it tries again.
fn poll_failed(json: bool, e: &fctl_client::Error) {
    if !json {
        eprintln!(
            "{}",
            style::dim(&format!("Reading the runner failed ({e}); trying again…"))
        );
    }
}

/// Stops what still runs on this runner, as `--force` does: asked again
/// every 2 seconds while the control plane doesn't answer, for `limit` at
/// most, so that one failed request doesn't leave agents unstopped as the
/// machine goes.
async fn stop_sessions(
    api: &fctl_client::Api,
    json: bool,
    limit: std::time::Duration,
) -> Result<RunnerDetail> {
    let deadline = std::time::Instant::now() + limit;
    loop {
        match api.drain_this_runner(true, true).await {
            Ok(d) => return Ok(d),
            Err(e) if std::time::Instant::now() < deadline => {
                if !json {
                    eprintln!(
                        "{}",
                        style::dim(&format!(
                            "Stopping the sessions failed ({e}); trying again…"
                        ))
                    );
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "stopping the sessions that still run here failed for {}: {e}",
                    duration_words(limit)
                ));
            }
        }
    }
}

/// Polls until no session runs on this runner, saying what changes.
async fn wait_until_drained(
    api: &fctl_client::Api,
    mut d: RunnerDetail,
    json: bool,
    timeout: Option<std::time::Duration>,
) -> Result<RunnerDetail> {
    let deadline = timeout.map(|t| std::time::Instant::now() + t);
    let key = |d: &RunnerDetail| -> Vec<(String, &'static str)> {
        d.sessions
            .iter()
            .map(|s| (s.id.0.clone(), s.status.as_str()))
            .collect()
    };
    if !json && !d.sessions.is_empty() {
        println!(
            "{}",
            style::dim(&match timeout {
                Some(t) => format!(
                    "Waiting until none runs here, for {} at most (Ctrl-C stops waiting; the runner keeps draining)…",
                    duration_words(t)
                ),
                None => {
                    "Waiting until none runs here (Ctrl-C stops waiting; the runner keeps draining)…"
                        .to_owned()
                }
            })
        );
    }
    // Polls that fail in a row: without a deadline, the wait gives up
    // after a minute of them; with one, they count as still running until
    // it, so that what still runs then stops all the same.
    let mut failed = 0;
    while !d.sessions.is_empty() {
        if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            return Ok(d);
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let next = match api.this_runner().await {
            Ok(next) => {
                failed = 0;
                next
            }
            Err(e) if deadline.is_some() || failed < 20 => {
                failed += 1;
                poll_failed(json, &e);
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        if !json && key(&next) != key(&d) && !next.sessions.is_empty() {
            println!(
                "{}",
                style::dim(&format!(
                    "{} left:",
                    match next.sessions.len() {
                        1 => "1 session".to_owned(),
                        n => format!("{n} sessions"),
                    }
                ))
            );
            admin::print_sessions(&next.sessions);
        }
        d = next;
    }
    if !json {
        println!("{} No sessions run here.", style::accent("✓"));
    }
    Ok(d)
}

/// How long the sessions a drain that ran out of time stops get to stop:
/// their agents push their work.
const STOP_AFTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// The longest `--timeout`: an autoscaling group's lifecycle hook, at its
/// longest, gives a machine 48 hours.
const MAX_TIMEOUT_HOURS: u64 = 48;

/// `--timeout`: `90s`, `10m`, `1h`, at most [`MAX_TIMEOUT_HOURS`].
fn timeout_arg(s: &str) -> std::result::Result<std::time::Duration, String> {
    let t = s.trim().to_lowercase();
    let (n, per) = if let Some(n) = t.strip_suffix('s') {
        (n, 1)
    } else if let Some(n) = t.strip_suffix('m') {
        (n, 60)
    } else if let Some(n) = t.strip_suffix('h') {
        (n, 3600)
    } else {
        return Err(format!("{s:?} needs a unit: 90s, 10m or 1h"));
    };
    match n
        .trim()
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(per))
    {
        Some(secs) if secs > MAX_TIMEOUT_HOURS * 3600 => Err(format!(
            "{s:?} is longer than a drain waits: at most {MAX_TIMEOUT_HOURS}h"
        )),
        Some(secs) if secs > 0 => Ok(std::time::Duration::from_secs(secs)),
        _ => Err(format!("{s:?} isn't a duration: use 90s, 10m or 1h")),
    }
}

/// `90 seconds`, `10 minutes`, `1 hour`.
fn duration_words(d: std::time::Duration) -> String {
    let s = d.as_secs();
    match s {
        1 => "1 second".into(),
        s if s < 60 || s % 60 != 0 => format!("{s} seconds"),
        60 => "1 minute".into(),
        s if s % 3600 != 0 => format!("{} minutes", s / 60),
        3600 => "1 hour".into(),
        s => format!("{} hours", s / 3600),
    }
}

/// `--retire-when-idle`: minutes, or 0 for never.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetireAfter(u32);

fn retire_arg(s: &str) -> std::result::Result<RetireAfter, String> {
    fctl_domain::enrollment::parse_minutes(s).map(|m| RetireAfter(m.unwrap_or(0)))
}

/// Whether and how this machine retires once idle, in a sentence.
fn retire_sentence(cfg: &HostConfig) -> Option<String> {
    let after = cfg.retire_after()?;
    let minutes =
        fctl_domain::enrollment::minutes(u32::try_from(after.as_secs() / 60).unwrap_or(u32::MAX));
    Some(match &cfg.retire_command {
        Some(c) => format!(
            "Once its runner has run nothing for {minutes}, this machine drains, runs `{c}`, then leaves and runs nothing more until installed again (retire_idle_minutes)."
        ),
        None => format!(
            "Once its runner has run nothing for {minutes}, this machine drains, leaves and runs nothing more until installed again (retire_idle_minutes)."
        ),
    })
}

/// `--memory 8g`: MiB, at least 512.
/// Where this host's sandboxes keep their files, as the daemon finds it,
/// and how much is free there.
async fn disk_here(cfg: &HostConfig, d: &Driver) -> (Option<DiskSpot>, Option<u64>) {
    let Some(spot) = d.disk_spot(&cfg.data_dir()).await else {
        return (None, None);
    };
    let free = d.disk_free_mb(&spot, &cfg.own_images()).await;
    (Some(spot), free)
}

/// `212 GiB free at …, keeps 20 GiB free`, or how far below that it is.
fn disk_line(spot: &DiskSpot, free: Option<u64>, min_free: Option<u64>) -> String {
    let Some(free) = free else {
        return style::dim(&format!("can't measure the free space at {spot}"));
    };
    let here = format!("{} free at {spot}", size::memory(free));
    match min_free {
        Some(min) if free < min => format!(
            "{here}, {}",
            style::amber(&format!(
                "below the {} it keeps free: it removes stopped sandboxes and takes no new sessions until there is room",
                size::memory(min)
            ))
        ),
        Some(min) => format!("{here}, keeps {} free", size::memory(min)),
        None => here,
    }
}

/// `2 projects, 3.4 GiB when last measured; up to 10 GiB each, …`: the
/// package caches this host keeps for the projects that share them.
fn caches_line(cfg: &HostConfig, driver: &Driver, cached: &caches::Records) -> String {
    let Some(max) = cfg.max_cache_mb() else {
        return style::dim("none (max_cache_gb = 0)");
    };
    if driver.cache_spot("prj_0", &cfg.data_dir()).is_none() {
        return style::dim(&format!("none with the {} driver", driver.name()));
    }
    let kept = match cfg.keep_caches_days {
        0 => "kept however long no session uses them".to_owned(),
        1 => "removed after a day without a session".to_owned(),
        days => format!("removed after {days} days without a session"),
    };
    let limits = format!("up to {} each, {kept}", size::memory(max));
    let taken: u64 = cached.projects.values().filter_map(|r| r.size_mb).sum();
    match cached.projects.len() {
        0 => format!("none yet; {limits}"),
        1 => format!(
            "1 project, {} when last measured; {limits}",
            size::memory(taken)
        ),
        n => format!(
            "{n} projects, {} when last measured; {limits}",
            size::memory(taken)
        ),
    }
}

fn memory_arg(s: &str) -> std::result::Result<u32, String> {
    match size::parse_memory_mb(s)? {
        mb if mb < 512 => Err("use at least 512 MiB".into()),
        mb => Ok(mb),
    }
}

/// How long `fctl host doctor` waits for each running sandbox's VM to
/// answer.
const ANSWER_WITHIN: std::time::Duration = std::time::Duration::from_secs(10);

/// What `fctl host doctor` says of the running container `name`, whose VM
/// didn't answer within [`ANSWER_WITHIN`], and how to stop it: kill `pids`,
/// the processes that run its VM.
fn not_answering(name: &str, pids: &[i32]) -> (String, String) {
    let what = format!(
        "{} {name} does not answer within {} seconds, which holds up every sandbox of Apple's `container`",
        if name.starts_with("fctl-") {
            "sandbox"
        } else {
            "container"
        },
        ANSWER_WITHIN.as_secs()
    );
    let now = match pids {
        [] => "`killall -9 container-apiserver; container system stop; container system start` stops every sandbox and starts Apple's `container` again".to_owned(),
        pids => {
            let pids: Vec<String> = pids.iter().map(i32::to_string).collect();
            format!("`kill -9 {}` (its container-runtime-linux) stops it now", pids.join(" "))
        }
    };
    let fix = if name.starts_with("fctl-") {
        format!("the host service stops it once a command waits on it; {now}")
    } else {
        format!("it isn't one of fctl's, so the host service leaves it: {now}")
    };
    (what, fix)
}

async fn doctor() -> Result<()> {
    let mut problems = 0;
    let mut check = |ok: bool, what: &str, fix: &str| {
        if ok {
            println!("{} {what}", style::accent("✓"));
        } else {
            problems += 1;
            println!("{} {what}", style::red("✕"));
            if !fix.is_empty() {
                println!("  {}", style::dim(fix));
            }
        }
    };
    let cfg = HostConfig::load();
    // The host service takes the group that lets it use Docker as it
    // starts; this does too, before it says anything, to find what the
    // service finds.
    if cfg.as_ref().is_ok_and(|c| uses_docker(&c.driver)) {
        docker_access::regroup();
    }
    check(cfg.is_ok(), "host configuration", "run `fctl host install`");
    let Ok(cfg) = cfg else {
        bail!("not configured")
    };
    check(
        HostConfig::load_token().is_ok(),
        "runner token",
        "run `fctl host install` again",
    );
    // The Claude token it keeps for its owner, if any: one from `claude
    // setup-token`, in a file only they read.
    let claude = cfg.claude_token();
    match claude_token::load(&claude) {
        Ok(None) => println!(
            "{} {}",
            style::dim("·"),
            style::dim(
                "Claude token: none kept here (`fctl host claude set-token` keeps yours here rather than in fctl)"
            )
        ),
        Ok(Some(k)) => check(
            claude_token::private(&claude),
            &format!(
                "Claude token: {} kept in {}",
                fctl_domain::claude::masked(&k.token),
                claude.display()
            ),
            &format!(
                "others on this machine can read it: chmod 600 {}",
                claude.display()
            ),
        ),
        Err(e) => check(
            false,
            "Claude token",
            &format!("{e:#}; `fctl host claude set-token` keeps a token there in its place"),
        ),
    }
    let api = fctl_client::Api::new(&cfg.server, None);
    let theirs = match api.instance().await {
        Ok(i) => {
            check(
                true,
                &format!("reaches {} ({} {})", cfg.server, i.name, i.version),
                "",
            );
            Some(i.version)
        }
        Err(e) => {
            check(false, &format!("reaches {}", cfg.server), &e.to_string());
            None
        }
    };
    let behind = theirs
        .as_deref()
        .and_then(|v| fctl_domain::updates::update_to(update::VERSION, v));
    let u = Updates::of(&cfg);
    match (u.why_not(), behind) {
        (None, None) => check(
            true,
            "updates: fctl updates itself to the version the control plane runs",
            "",
        ),
        (None, Some(v)) => check(
            true,
            &format!("updates: fctl updates itself to {v}, which the control plane runs"),
            "",
        ),
        (Some(why), behind) => {
            let behind = behind
                .map(|v| format!("; the control plane runs fctl {v}: `fctl self update`"))
                .unwrap_or_default();
            println!(
                "{} {}",
                style::dim("·"),
                style::dim(&format!("updates: {why}{behind}"))
            );
        }
    }
    let docker = uses_docker(&cfg.driver);
    for d in Driver::probe_all(&cfg).await {
        let line = format!(
            "driver {}: {}",
            d.name,
            d.detail.clone().unwrap_or_default()
        );
        if d.name == cfg.driver || (cfg.driver == "auto" && d.available) {
            check(d.available, &line, "");
        } else {
            println!("{} {}", style::dim("·"), style::dim(&line));
        }
    }
    // With Apple's `container`, one VM that doesn't answer holds up every
    // sandbox: the host service stops fctl's once a command waits on one.
    if let Ok(d) = Driver::select(&cfg.driver, &cfg).await
        && let Some(Ok(stuck)) = d.not_answering(ANSWER_WITHIN).await
    {
        if stuck.is_empty() {
            check(true, "sandboxes answer", "");
        }
        for (name, pids) in stuck {
            let (what, fix) = not_answering(&name, &pids);
            check(false, &what, &fix);
        }
    }
    if docker
        && let Some(pid) = service::main_pid()
        && let Some(has) = docker_access::process_has_group(pid)
    {
        check(
            has,
            "the host service has the group that lets it use Docker",
            &format!(
                "it started without it and couldn't take it: `{}` (or restarting the machine) gives it",
                docker_access::restart_manager()
            ),
        );
    }
    if let Ok(d) = Driver::select(&cfg.driver, &cfg).await
        && d.uses_images()
        && d.probe().await.available
    {
        let image = cfg.image();
        if let Ok(Some(here)) = d.image(&image).await {
            let detail = if refresh::published(&here) {
                refresh::claude_code(&here)
            } else if cfg.image.is_none() {
                " (built here: the host doesn't pull over it)".into()
            } else {
                String::new()
            };
            check(true, &format!("sandbox image {image}{detail}"), "");
        } else {
            let meanwhile = match cfg.fallback_image() {
                Some(f) if d.has_image(&f).await => {
                    format!("; sessions start from {f} until it is")
                }
                _ => String::new(),
            };
            println!(
                "{} {}",
                style::dim("·"),
                style::dim(&format!(
                    "sandbox image {image}: not here yet{meanwhile} (the host pulls it when it starts; `fctl host image pull` pulls it now)"
                ))
            );
        }
    }
    let problems_with_images = cfg.image_problems();
    check(
        problems_with_images.is_empty(),
        &format!(
            "images: {}; {}",
            image::describe_allowed(cfg.allowed_images.as_deref()),
            image::describe_builds(cfg.allowed_builds.as_deref())
        ),
        &problems_with_images.join("; "),
    );
    if cfg.allowed_builds.is_some()
        && let Ok(d) = Driver::select(&cfg.driver, &cfg).await
        && !d.builds_images()
    {
        println!(
            "{} {}",
            style::dim("·"),
            style::dim(&format!(
                "allowed_builds: the {} driver builds no images, so this runner builds none",
                d.name()
            ))
        );
    }
    if let Ok(d) = Driver::select(&cfg.driver, &cfg).await
        && let (Some(spot), free) = disk_here(&cfg, &d).await
    {
        match (free, cfg.min_free_disk_mb()) {
            (Some(free), Some(min)) => {
                let low = free < min;
                let (free, min) = (size::memory(free), size::memory(min));
                check(
                    !low,
                    &if low {
                        format!("disk: {free} free at {spot}, below the {min} it keeps free")
                    } else {
                        format!("disk: {free} free at {spot}, keeps {min} free")
                    },
                    "free some space there, or lower min_free_disk_gb in host.toml; until then the host takes no new sessions",
                );
            }
            (Some(_), None) => println!(
                "{} {}",
                style::dim("·"),
                style::dim(&format!(
                    "disk: {} (min_free_disk_gb = 0)",
                    disk_line(&spot, free, None)
                ))
            ),
            (None, _) => println!(
                "{} {}",
                style::dim("·"),
                style::dim(&format!("disk: can't measure the free space at {spot}"))
            ),
        }
    }
    let (installed, running) = service::status();
    check(
        installed && running,
        "service running",
        "run `fctl host install` (or `fctl host restart`)",
    );
    if cfg!(target_os = "macos") {
        let pm = std::process::Command::new("pmset")
            .arg("-g")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let sleep_off = pm.lines().any(|l| {
            l.trim_start().starts_with("sleep") && l.split_whitespace().nth(1) == Some("0")
        });
        check(
            sleep_off,
            "system sleep disabled",
            "sudo pmset -a sleep 0 disksleep 0 standby 0 autorestart 1",
        );
    }
    if problems == 0 {
        println!("\n{}", style::accent("Everything looks good."));
        Ok(())
    } else {
        bail!("{problems} problem(s) found")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A runner's detail as the control plane answers it, with `sessions`
    /// running.
    fn detail(sessions: u32) -> Result<RunnerDetail> {
        let running: Vec<serde_json::Value> = (0..sessions)
            .map(|i| {
                serde_json::json!({
                    "id": format!("ses_{i}"), "project_slug": "web", "number": i,
                    "title": "Work", "status": "running",
                })
            })
            .collect();
        Ok(serde_json::from_value(serde_json::json!({
            "runner": {
                "id": "rnr_spot", "name": "spot", "status": "online", "drivers": [],
                "capacity": 2, "active_sessions": sessions, "labels": [], "created_at": 0,
                "yours": true,
            },
            "sessions": running,
        }))?)
    }

    /// A stand-in for the control plane: each request gets the next answer
    /// queued for its method and path, `(status, body)`, and the last one
    /// again once they run out; others get a 404.
    async fn stand_in(answers: Vec<(&str, Vec<(u16, String)>)>) -> Result<String> {
        use std::collections::{HashMap, VecDeque};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let answers: HashMap<String, VecDeque<(u16, String)>> = answers
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.into()))
            .collect();
        let answers = std::sync::Arc::new(std::sync::Mutex::new(answers));
        tokio::spawn(async move {
            while let Ok((mut conn, _)) = listener.accept().await {
                let answers = answers.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    // The head, then as much of the body as it says.
                    let end = loop {
                        let n = match conn.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => n,
                        };
                        buf.extend(chunk.iter().take(n));
                        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(buf.get(..end).unwrap_or_default());
                    let length = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    let key = head.split(' ').take(2).collect::<Vec<_>>().join(" ");
                    while buf.len() < end + length {
                        match conn.read(&mut chunk).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => buf.extend(chunk.iter().take(n)),
                        }
                    }
                    let (status, body) = {
                        let Ok(mut answers) = answers.lock() else {
                            return;
                        };
                        match answers.get_mut(&key) {
                            Some(q) if q.len() > 1 => q.pop_front(),
                            Some(q) => q.front().cloned(),
                            None => None,
                        }
                        .unwrap_or((404, "{}".into()))
                    };
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = conn.write_all(reply.as_bytes()).await;
                    let _ = conn.shutdown().await;
                });
            }
        });
        Ok(format!("http://{addr}"))
    }

    /// A drain with a deadline stops what still runs at the deadline,
    /// though every poll before it failed; asks again when the stop fails;
    /// and waits for the sessions to stop through a poll that fails.
    #[tokio::test]
    async fn a_drain_with_a_deadline_stops_its_sessions_through_failed_requests() -> Result<()> {
        use std::time::Duration;
        let down = (
            503,
            r#"{"error":{"code":"unavailable","message":"Down"}}"#.to_owned(),
        );
        let server = stand_in(vec![
            (
                "GET /api/v1/runner",
                vec![
                    down.clone(),
                    down.clone(),
                    (200, serde_json::to_string(&detail(0)?)?),
                ],
            ),
            (
                "POST /api/v1/runner/drain",
                vec![down, (200, serde_json::to_string(&detail(1)?)?)],
            ),
        ])
        .await?;
        let api = fctl_client::Api::new(&server, Some("fctl_rt_test".into()));
        let d = wait_until_drained(&api, detail(1)?, true, Some(Duration::from_secs(1))).await?;
        assert_eq!(d.sessions.len(), 1);
        let stopping = stop_sessions(&api, true, Duration::from_secs(10)).await?;
        assert_eq!(stopping.sessions.len(), 1);
        let d = wait_until_stopped(&api, stopping, true, Duration::from_secs(10)).await;
        assert!(d.sessions.is_empty());
        Ok(())
    }

    #[test]
    fn drains_wait_and_machines_retire_as_long_as_they_are_told() {
        use std::time::Duration;
        assert_eq!(timeout_arg("90s"), Ok(Duration::from_secs(90)));
        assert_eq!(timeout_arg("10m"), Ok(Duration::from_secs(600)));
        assert_eq!(timeout_arg(" 2H "), Ok(Duration::from_secs(7200)));
        assert!(timeout_arg("90").is_err());
        assert!(timeout_arg("0s").is_err());
        assert_eq!(timeout_arg("48h"), Ok(Duration::from_secs(48 * 3600)));
        assert!(timeout_arg("49h").is_err());
        assert!(timeout_arg("18446744073709551615h").is_err());
        assert_eq!(duration_words(Duration::from_secs(90)), "90 seconds");
        assert_eq!(duration_words(Duration::from_secs(60)), "1 minute");
        assert_eq!(duration_words(Duration::from_secs(600)), "10 minutes");
        assert_eq!(duration_words(Duration::from_secs(7200)), "2 hours");
        assert_eq!(retire_arg("20m"), Ok(RetireAfter(20)));
        assert_eq!(retire_arg("1h"), Ok(RetireAfter(60)));
        assert_eq!(retire_arg("never"), Ok(RetireAfter(0)));
        let mut cfg = HostConfig::default();
        assert_eq!(retire_sentence(&cfg), None);
        cfg.retire_idle_minutes = 20;
        cfg.retire_command = Some("systemctl poweroff".into());
        assert_eq!(
            retire_sentence(&cfg).as_deref(),
            Some(
                "Once its runner has run nothing for 20 minutes, this machine drains, runs `systemctl poweroff`, then leaves and runs nothing more until installed again (retire_idle_minutes)."
            )
        );
    }

    #[test]
    fn doctor_says_which_vm_holds_up_apples_container_and_how_to_stop_it() {
        assert_eq!(
            not_answering("fctl-ses-01m7rm7hkxza3q8xpzzbx1prda", &[72625]),
            (
                "sandbox fctl-ses-01m7rm7hkxza3q8xpzzbx1prda does not answer within 10 seconds, which holds up every sandbox of Apple's `container`".to_owned(),
                "the host service stops it once a command waits on it; `kill -9 72625` (its container-runtime-linux) stops it now".to_owned()
            )
        );
        let (what, fix) = not_answering("dev", &[]);
        assert!(what.starts_with("container dev does not answer"), "{what}");
        assert_eq!(
            fix,
            "it isn't one of fctl's, so the host service leaves it: `killall -9 container-apiserver; container system stop; container system start` stops every sandbox and starts Apple's `container` again"
        );
    }

    #[test]
    fn only_a_line_that_says_no_says_no() -> Result<()> {
        let answer = |input: &str| yes_unless_no(&mut input.as_bytes());
        assert_eq!(answer("\n")?, Some(true));
        assert_eq!(answer("y\n")?, Some(true));
        assert_eq!(answer("n\n")?, Some(false));
        assert_eq!(answer(" No \n")?, Some(false));
        // Ctrl-D, or no input at all, runs nothing.
        assert_eq!(answer("")?, None);
        Ok(())
    }

    fn probed(name: &str, available: bool, detail: &str) -> DriverInfo {
        DriverInfo {
            name: name.into(),
            available,
            isolation: "vm".into(),
            version: None,
            detail: Some(detail.into()),
        }
    }

    /// What `fctl host install` heard on a new Ubuntu VM (fctl#345), with
    /// Firecracker's probe as it would be.
    fn ubuntu(firecracker: DriverInfo) -> Vec<DriverInfo> {
        vec![
            probed(
                "apple",
                false,
                "Apple container needs macOS 26 on Apple silicon",
            ),
            probed("docker", false, "the docker CLI is not installed"),
            firecracker,
            probed("lima", false, "limactl is not installed"),
            probed("process", true, "no isolation"),
        ]
    }

    #[test]
    fn says_what_firecracker_lacks_on_linux() {
        let why = no_runtime(
            &ubuntu(probed("firecracker", false, "this machine has no /dev/kvm")),
            "linux",
        );
        assert_eq!(
            why,
            "No VM or container runtime is ready on this machine:\
            \n  docker: the docker CLI is not installed\
            \n  firecracker: this machine has no /dev/kvm\
            \n  lima: limactl is not installed\
            \nInstall one (fctl uses Firecracker only with --driver firecracker), or pass --driver process to run agents without isolation."
        );
    }

    #[test]
    fn points_to_firecracker_when_only_it_is_ready() {
        let why = no_runtime(&ubuntu(probed("firecracker", true, "ready")), "linux");
        assert_eq!(
            why,
            "Only Firecracker is ready on this machine, and fctl uses it only when asked:\
            \n  docker: the docker CLI is not installed\
            \n  firecracker: ready\
            \n  lima: limactl is not installed\
            \nPass --driver firecracker to run each session in a microVM of its own, or install another."
        );
    }

    #[test]
    fn leaves_firecracker_out_on_a_mac() {
        let why = no_runtime(
            &[
                probed("apple", false, "container is not installed"),
                probed("docker", false, "the docker CLI is not installed"),
                probed(
                    "firecracker",
                    false,
                    "Firecracker needs a Linux host with KVM",
                ),
                probed("lima", false, "limactl is not installed"),
                probed("process", true, "no isolation"),
            ],
            "macos",
        );
        assert_eq!(
            why,
            "No VM or container runtime is ready on this machine:\
            \n  apple: container is not installed\
            \n  docker: the docker CLI is not installed\
            \n  lima: limactl is not installed\
            \nInstall one, or pass --driver process to run agents without isolation."
        );
    }
}
