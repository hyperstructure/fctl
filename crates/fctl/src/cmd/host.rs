//! `fctl host …`: run this machine as a runner.

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::Subcommand;
use fctl_domain::size;
use fctl_host::config::means_default_image;
use fctl_host::driver::Driver;
use fctl_host::{HostConfig, service};

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
    /// Restart the service (sessions keep running).
    Restart,
    /// Stop and remove the service (sandboxes are kept).
    Uninstall,
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
            token,
            no_service,
            build_image,
        } => {
            const SIGN_IN: &str = "Sign in as an admin first (`fctl login`), or pass --token";
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
                fctl_host::image::build(&d, &cfg.image(), &cfg.download_base).await?;
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
                    let reg = api.context(SIGN_IN)?.create_runner(&cfg.name).await?;
                    println!(
                        "{} Registered runner {} ({})",
                        style::accent("✓"),
                        style::bold(&reg.runner.name),
                        reg.runner.id
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
            } else if !build_image && d.needs_linux_agent() {
                println!(
                    "{}",
                    style::dim("Prepare the base VM once with `fctl host image build`.")
                );
            }
            if !no_service {
                let exe = std::env::current_exe()?;
                let what = service::install(&exe)?;
                println!("{} Service: {what}", style::accent("✓"));
                println!(
                    "{}",
                    style::dim(&format!("Logs: {}", service::log_path().display()))
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
            let sandbox = match Driver::select(&cfg.driver, &cfg.data_dir(), None).await {
                Ok(d) => Some(cfg.sandbox_limits(d.resources().await)),
                Err(_) => None,
            };
            if json {
                return style::print_json(
                    &serde_json::json!({ "config": cfg, "image": cfg.image(), "sandbox": sandbox, "service_installed": installed, "service_running": running }),
                );
            }
            println!("{} {}", style::bold(&cfg.name), style::dim(&cfg.server));
            println!("driver:   {}", cfg.driver);
            println!("capacity: {} sessions", cfg.capacity);
            if let Some(l) = &sandbox {
                println!("sandbox:  {}", size::limits(l));
            }
            println!("image:    {}", cfg.image());
            println!(
                "service:  {}",
                match (installed, running) {
                    (true, true) => style::accent("running"),
                    (true, false) => style::amber("installed, not running"),
                    _ => style::dim("not installed"),
                }
            );
            Ok(())
        }
        HostCmd::Doctor => doctor().await,
        HostCmd::Logs { lines } => {
            let path = service::log_path();
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let all: Vec<&str> = text.lines().collect();
            for l in &all[all.len().saturating_sub(lines)..] {
                println!("{l}");
            }
            Ok(())
        }
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
            d.pull(&image, !json).await?;
            if json {
                return style::print_json(&serde_json::json!({ "image": image }));
            }
            println!("{} Pulled {image}", style::accent("✓"));
            Ok(())
        }
        HostCmd::Image(ImageCmd::Build { driver, tag }) => {
            let cfg = HostConfig::load().unwrap_or_default();
            let name = driver.unwrap_or(cfg.driver.clone());
            let d = Driver::select(&name, &cfg.data_dir(), cfg.agent_bin.clone()).await?;
            let tag = tag.unwrap_or_else(|| cfg.image());
            fctl_host::image::build(&d, &tag, &cfg.download_base).await
        }
        HostCmd::Restart => {
            service::restart()?;
            println!("Restarted. Sessions keep running in their sandboxes.");
            Ok(())
        }
        HostCmd::Uninstall => {
            service::uninstall()?;
            println!("Removed the service. Configuration and sandboxes are kept.");
            Ok(())
        }
    }
}

/// `--memory 8g`: MiB, at least 512.
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
    let api = fctl_client::Api::new(&cfg.server, None);
    match api.instance().await {
        Ok(i) => check(
            true,
            &format!("reaches {} ({} {})", cfg.server, i.name, i.version),
            "",
        ),
        Err(e) => check(false, &format!("reaches {}", cfg.server), &e.to_string()),
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
        if d.has_image(&image).await {
            check(true, &format!("sandbox image {image}"), "");
        } else {
            println!(
                "{} {}",
                style::dim("·"),
                style::dim(&format!(
                    "sandbox image {image}: not here yet (the host pulls it when it starts; `fctl host image pull` pulls it now)"
                ))
            );
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
