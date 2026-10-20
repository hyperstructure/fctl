//! `cargo xtask serve`: the standalone control plane from this checkout, as
//! its container image runs it (deploy/standalone, docs/self-hosting.md): the
//! Worker and the web app built for production, run by workerd through
//! Miniflare, with D1 and the Durable Objects in a directory. No Cloudflare
//! account. Its settings are the image's environment variables; the flags
//! set the common ones.

use crate::util::{self, run};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct ServeOpts {
    /// `FCTL_PUBLIC_URL`: the address people and hosts reach it at.
    pub public_url: Option<String>,
    /// `FCTL_PORT`.
    pub port: Option<String>,
    /// `FCTL_LISTEN`.
    pub listen: Option<String>,
    /// `FCTL_DATA_DIR` (default .fctl-serve/).
    pub data: Option<PathBuf>,
    /// Serve what the last build left in crates/fctl-control/build and web/dist.
    pub skip_build: bool,
}

pub fn serve(root: &Path, opts: ServeOpts) -> Result<()> {
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let port = opts
        .port
        .or_else(|| env("FCTL_PORT"))
        .unwrap_or_else(|| "8787".to_owned());
    let public_url = opts
        .public_url
        .or_else(|| env("FCTL_PUBLIC_URL"))
        .unwrap_or_else(|| format!("http://localhost:{port}"));
    // Only this machine, unless others are to reach it.
    let listen = opts
        .listen
        .or_else(|| env("FCTL_LISTEN"))
        .unwrap_or_else(|| {
            if is_local(&public_url) {
                "127.0.0.1".to_owned()
            } else {
                "0.0.0.0".to_owned()
            }
        });
    let data = opts
        .data
        .or_else(|| env("FCTL_DATA_DIR").map(PathBuf::from))
        .unwrap_or_else(|| root.join(".fctl-serve"));

    if !root.join("node_modules").exists() {
        util::npm_install(root)?;
    }
    let control = root.join("crates/fctl-control");
    if opts.skip_build {
        if !control.join("build/index.js").exists() || !root.join("web/dist/index.html").exists() {
            bail!("nothing built yet: run cargo xtask serve without --skip-build once");
        }
    } else {
        util::build_web(root)?;
        run(util::worker_build()?.arg("--release").current_dir(&control))?;
    }
    if !util::has("node") {
        bail!("node is not installed (Node.js 22 or newer runs the control plane)");
    }
    std::fs::create_dir_all(&data)?;
    let data = data
        .canonicalize()
        .with_context(|| format!("{}", data.display()))?;
    // Ctrl-C reaches node too, which stops the server and its runtime.
    run(Command::new("node")
        .arg(root.join("deploy/standalone/server.mjs"))
        .env("FCTL_PUBLIC_URL", &public_url)
        .env("FCTL_PORT", &port)
        .env("FCTL_LISTEN", &listen)
        .env("FCTL_DATA_DIR", &data)
        .current_dir(root))
}

/// Whether `url` is on this machine (localhost or a loopback address).
fn is_local(url: &str) -> bool {
    let authority = url
        .split("://")
        .nth(1)
        .unwrap_or_default()
        .split('/')
        .next()
        .unwrap_or_default();
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    host == "localhost" || host.ends_with(".localhost") || host.starts_with("127.") || host == "::1"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_urls() {
        assert!(is_local("http://localhost:8787"));
        assert!(is_local("http://127.0.0.1:8787"));
        assert!(is_local("http://[::1]:8787"));
        assert!(!is_local("http://192.168.1.20:8787"));
        assert!(!is_local("https://fctl.example.com"));
    }
}
