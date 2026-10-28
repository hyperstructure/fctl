//! `fctl host …`: run this machine as a runner.

use crate::cmd::admin;
use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::Subcommand;
use clap::builder::BoolishValueParser;
use fctl_domain::{disk, image, size};
use fctl_host::config::{NOT_A_RUNNER, means_default_image};
use fctl_host::driver::{DiskSpot, Driver};
use fctl_host::{HostConfig, caches, claude_token, refresh, service, update};
use fctl_proto::api::{Runner, RunnerDetail, RunnerStatus};
use std::io::IsTerminal;

#[derive(Subcommand)]
pub enum HostCmd {
    /// Register this machine as a runner and start it as a service.
    Install {
        /// Runner name (default: the hostname).
        #[arg(long)]
        name: Option<String>,
        /// apple, docker, lima, process, or auto.
        #[arg(long, default_value = "auto")]
        driver: String,
        /// Maximum concurrent sessions.
        #[arg(long, default_value_t = 4)]
        capacity: u32,
        /// CPUs for a sandbox whose project sets none (new hosts: 4).
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        cpus: Option<u32>,
        /// Memory for a sandbox whose project sets none, like 8g or 8192m (new hosts: 8g).
        #[arg(long, value_parser = memory_arg)]
        memory: Option<u32>,
        /// The most CPUs one sandbox may have (new hosts: all of the machine's or the Docker VM's).
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        max_cpus: Option<u32>,
        /// The most memory one sandbox may have, like 16g (new hosts: all of the machine's or the Docker VM's).
        #[arg(long, value_parser = memory_arg)]
        max_memory: Option<u32>,
        /// Sandbox image for sessions whose project sets none (default: the
        /// published image of this version; "" goes back to it).
        #[arg(long)]
        image: Option<String>,
        /// An image this host runs for projects that choose their own: a
        /// reference, or a prefix like ghcr.io/acme/* (repeat it for more;
        /// it replaces the list, and "*" allows any). Without the list, any
        /// image runs.
        #[arg(long = "allow-image", value_name = "REF")]
        allow_images: Vec<String>,
        /// A repository, owner/name or owner/*, whose Dockerfile or
        /// devcontainer.json this host builds images from (repeat it for
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
        /// Write the configuration but do not install a service.
        #[arg(long)]
        no_service: bool,
        /// Also build the sandbox image here (the Lima base VM, or for a host
        /// that can't pull the published image).
        #[arg(long)]
        build_image: bool,
        /// on or off: whether the host updates fctl to the version the
        /// control plane runs, by itself (new hosts: on).
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        auto_update: Option<bool>,
    },
    /// Run the host in the foreground (what the service runs).
    Run,
    /// Show the host's configuration and service state.
    Status,
    /// Check that everything a host needs is in place.
    Doctor,
    /// Print the host's log.
    Logs {
        #[arg(short = 'n', long, default_value_t = 100)]
        lines: usize,
    },
    /// Pull the sandbox image, or build it with this host's driver.
    #[command(subcommand)]
    Image(ImageCmd),
    /// Keep your Claude token on this runner rather than in fctl.
    #[command(subcommand)]
    Claude(super::host_claude::HostClaudeCmd),
    /// Restart the service (sessions keep running).
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
    /// Take no new sessions; the ones here carry on, and a message resumes
    /// them here.
    Drain {
        /// Wait until no session runs here.
        #[arg(long)]
        wait: bool,
    },
    /// Take new sessions again.
    Undrain,
    /// Stop and remove the service (sandboxes are kept). When sessions run
    /// here, it says what happens to them and asks first.
    Uninstall {
        /// Drain this host first, and uninstall once no session runs here.
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
    /// Pull the host's sandbox image now (again: for a newer build of it).
    Pull,
    /// Build the default image here instead of pulling it (or the Lima base VM).
    Build {
        /// apple, docker or lima (default: the host's driver).
        #[arg(long)]
        driver: Option<String>,
        /// Image tag (default: the host's image, so that sessions use the build).
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
            let token = HostConfig::load_token()?;
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
            no_service,
            build_image,
            auto_update,
        } => {
            const SIGN_IN: &str = "Sign in first (`fctl login`), or pass --token";
            let server = config::server(server)?;
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
            if let Some(problem) = cfg.image_problems().first() {
                bail!("{problem}");
            }
            // Registering a runner leaves something behind, so it waits until
            // everything that can fail on this machine has passed.
            let api = if token.is_some() {
                None
            } else {
                Some(config::api(Some(&server)).context(SIGN_IN)?)
            };
            // `auto` picks like the daemon does, except that it never settles for
            // the process driver.
            let driver = if driver == "auto" {
                let infos = Driver::probe_all(&cfg.data_dir()).await;
                let runtimes = infos.iter().filter(|i| i.name != "process");
                match runtimes.clone().find(|i| i.available) {
                    Some(info) => info.name.clone(),
                    None => {
                        let mut why =
                            String::from("No VM or container runtime is ready on this machine:");
                        for info in runtimes {
                            let detail = info.detail.as_deref().unwrap_or_default();
                            why.push_str(&format!("\n  {}: {detail}", info.name));
                        }
                        bail!(
                            "{why}\nInstall one, or pass --driver process to run agents without isolation."
                        );
                    }
                }
            } else {
                driver
            };
            let d = Driver::select(&driver, &cfg.data_dir(), cfg.agent_bin.clone()).await?;
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
            let token = match token {
                Some(t) => t,
                None => {
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
                let images = cfg.images(true);
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
            if !no_service {
                let exe = std::env::current_exe()?;
                let what = service::install(&exe)?;
                println!("{} Service: {what}", style::accent("✓"));
                println!(
                    "{}",
                    style::dim(&format!("Logs: {}", service::log_location()))
                );
            }
            if matches!(claude_token::load(&cfg.claude_token()), Ok(None)) {
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
            let (installed, running) = service::status();
            let driver = Driver::select(&cfg.driver, &cfg.data_dir(), None)
                .await
                .ok();
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
                    &serde_json::json!({ "config": cfg, "image": cfg.image(), "sandbox": sandbox, "disk": disk, "caches": caches, "updates": updates, "claude_token": claude, "service_installed": installed, "service_running": running, "runner": runner.as_ref().ok() }),
                );
            }
            println!("{} {}", style::bold(&cfg.name), style::dim(&cfg.server));
            if let Ok(d) = &runner {
                println!("id:       {}", d.runner.id);
            }
            println!("driver:   {}", cfg.driver);
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
            println!("claude:   {}", claude_line(&cfg));
            if let Some(d) = &driver {
                let images = cfg.images(d.uses_images());
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
                    match admin::drain_line(&d.runner) {
                        Some(drain) => println!("runner:   {state}, {}", style::amber(&drain)),
                        None => println!("runner:   {state}"),
                    }
                    if !d.sessions.is_empty() {
                        println!("sessions:");
                        admin::print_sessions(&d.sessions);
                    }
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
            let d = Driver::select(&cfg.driver, &cfg.data_dir(), cfg.agent_bin.clone()).await?;
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
                        "Sessions start from it until {image} can be pulled: a release's image is published within the hour after the release."
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
            let d = Driver::select(&name, &cfg.data_dir(), cfg.agent_bin.clone()).await?;
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
        HostCmd::Drain { wait } => {
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
                d = wait_until_drained(&api, d, json).await?;
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
    match asked {
        Ok((_, d)) if d.sessions.is_empty() => {}
        Ok((api, d)) => {
            let n = d.sessions.len();
            let online = d.runner.status == RunnerStatus::Online;
            if !json {
                println!(
                    "{} on this host:",
                    match n {
                        1 => "1 session runs".to_owned(),
                        n => format!("{n} sessions run"),
                    }
                );
                admin::print_sessions(&d.sessions);
                println!(
                    "{}",
                    style::dim(if wait {
                        "Draining the host: no new sessions start here, and it uninstalls once none runs."
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
                wait_until_drained(&api, d, json).await?;
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
                        "{sessions} on this host. `fctl host uninstall --wait` lets {them} finish first; `--force` stops {them} now."
                    );
                }
                let d = api.drain_this_runner(true, true).await?;
                if online {
                    wait_until_stopped(&api, d, json).await?;
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
    if json {
        return style::print_json(&serde_json::json!({ "uninstalled": true, "drained": drained }));
    }
    println!("Removed the service. Configuration and sandboxes are kept.");
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

/// Asks a yes-or-no question on the terminal.
fn confirm(question: &str) -> Result<bool> {
    use std::io::{BufRead, Write};
    print!("{question}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "Yes"))
}

/// Waits, for two minutes at most, until the sessions this runner was asked
/// to stop have stopped.
async fn wait_until_stopped(api: &fctl_client::Api, mut d: RunnerDetail, json: bool) -> Result<()> {
    if !json {
        println!("{}", style::dim("Stopping…"));
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !d.sessions.is_empty() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        d = api.this_runner().await?;
    }
    if !json {
        match d.sessions.len() {
            0 => println!("{} No sessions run here.", style::accent("✓")),
            1 => println!(
                "{}",
                style::amber(
                    "A session hadn't stopped after two minutes; uninstalling anyway. Its agent stops on its own once it is idle."
                )
            ),
            n => println!(
                "{}",
                style::amber(&format!(
                    "{n} sessions hadn't stopped after two minutes; uninstalling anyway. Their agents stop on their own once they are idle."
                ))
            ),
        }
    }
    Ok(())
}

/// The sandboxes of sessions that run on this machine, by the host's driver.
async fn running_sandboxes() -> usize {
    let Ok(cfg) = HostConfig::load() else {
        return 0;
    };
    let Ok(d) = Driver::select(&cfg.driver, &cfg.data_dir(), cfg.agent_bin.clone()).await else {
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
    let Ok(d) = Driver::select(&cfg.driver, &cfg.data_dir(), cfg.agent_bin.clone()).await else {
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
    const NAME_IT: &str = "Which runner? This machine isn't one: give the runner's name or id (`fctl admin runners` lists them)";
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

/// Polls until no session runs on this runner, saying what changes.
async fn wait_until_drained(
    api: &fctl_client::Api,
    mut d: RunnerDetail,
    json: bool,
) -> Result<RunnerDetail> {
    let key = |d: &RunnerDetail| -> Vec<(String, &'static str)> {
        d.sessions
            .iter()
            .map(|s| (s.id.0.clone(), s.status.as_str()))
            .collect()
    };
    if !json && !d.sessions.is_empty() {
        println!(
            "{}",
            style::dim(
                "Waiting until none runs here (Ctrl-C stops waiting; the host keeps draining)…"
            )
        );
    }
    while !d.sessions.is_empty() {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let next = api.this_runner().await?;
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
    for d in Driver::probe_all(&cfg.data_dir()).await {
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
    if let Ok(d) = Driver::select(&cfg.driver, &cfg.data_dir(), None).await
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
        && let Ok(d) = Driver::select(&cfg.driver, &cfg.data_dir(), None).await
        && !d.uses_images()
    {
        println!(
            "{} {}",
            style::dim("·"),
            style::dim(&format!(
                "allowed_builds: the {} driver uses no images, so this host builds none",
                d.name()
            ))
        );
    }
    if let Ok(d) = Driver::select(&cfg.driver, &cfg.data_dir(), None).await
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
