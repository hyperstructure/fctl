//! `fctl host …`: run this machine as a runner.

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::Subcommand;
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
        /// Sandbox image.
        #[arg(long)]
        image: Option<String>,
        /// Use an existing runner token instead of registering a new runner.
        #[arg(long, env = "FCTL_RUNNER_TOKEN", hide_env_values = true)]
        token: Option<String>,
        /// Write the configuration but do not install a service.
        #[arg(long)]
        no_service: bool,
        /// Also build the sandbox image now.
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
    /// Build the default sandbox image with this host's driver.
    #[command(subcommand)]
    Image(ImageCmd),
    /// Restart the service (sessions keep running).
    Restart,
    /// Stop and remove the service (sandboxes are kept).
    Uninstall,
}

#[derive(Subcommand)]
pub enum ImageCmd {
    /// Build `fctl-agent:latest` (or the Lima base VM).
    Build {
        /// apple, docker or lima (default: the host's driver).
        #[arg(long)]
        driver: Option<String>,
        /// Image tag (default: the host's image, fctl-agent:latest).
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
            image,
            token,
            no_service,
            build_image,
        } => {
            let server = config::server(server)?;
            let mut cfg = HostConfig::load().unwrap_or_default();
            cfg.server = server.clone();
            if let Some(n) = name {
                cfg.name = n;
            }
            cfg.driver = driver;
            cfg.capacity = capacity.max(1);
            if let Some(i) = image {
                cfg.image = i;
            }
            let token = match token {
                Some(t) => t,
                None => {
                    let api = config::api(Some(&server))
                        .context("Sign in as an admin first (`fctl login`), or pass --token")?;
                    let reg = api.create_runner(&cfg.name).await?;
                    println!(
                        "{} Registered runner {} ({})",
                        style::accent("✓"),
                        style::bold(&reg.runner.name),
                        reg.runner.id
                    );
                    reg.token
                }
            };
            cfg.save()?;
            HostConfig::save_token(&token)?;
            println!(
                "{} Wrote {}",
                style::accent("✓"),
                HostConfig::path().display()
            );
            let d = Driver::select(&cfg.driver, &cfg.data_dir(), cfg.agent_bin.clone()).await?;
            let info = d.probe().await;
            println!(
                "{} Driver: {} ({})",
                style::accent("✓"),
                info.name,
                info.detail.unwrap_or_default()
            );
            if build_image {
                fctl_host::image::build(&d, &cfg.image).await?;
            } else if d.needs_linux_agent() {
                println!(
                    "{}",
                    style::dim(&format!(
                        "Build the sandbox image once with `fctl host image build` (tag {}).",
                        cfg.image
                    ))
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
            if json {
                return style::print_json(
                    &serde_json::json!({ "config": cfg, "service_installed": installed, "service_running": running }),
                );
            }
            println!("{} {}", style::bold(&cfg.name), style::dim(&cfg.server));
            println!("driver:   {}", cfg.driver);
            println!(
                "capacity: {} sessions · {} CPUs · {} MiB each",
                cfg.capacity, cfg.cpus, cfg.memory_mb
            );
            println!("image:    {}", cfg.image);
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
        HostCmd::Image(ImageCmd::Build { driver, tag }) => {
            let cfg = HostConfig::load().unwrap_or_default();
            let name = driver.unwrap_or(cfg.driver.clone());
            let d = Driver::select(&name, &cfg.data_dir(), cfg.agent_bin.clone()).await?;
            fctl_host::image::build(&d, tag.as_deref().unwrap_or(&cfg.image)).await
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
