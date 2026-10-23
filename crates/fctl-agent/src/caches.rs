//! The project's package caches, which the host keeps for the project's
//! sessions and mounts in the sandbox (`AgentConfig::cache`): fctl-agent
//! points the package managers there, for the setup script and for Claude
//! and what it runs.
//!
//! npm, pnpm 11, Yarn, Bun, uv, pip, Go and Playwright take a variable.
//! pnpm 9 and 10 read theirs from `~/.config/pnpm/rc`, as npm warns about
//! variables it doesn't know. Cargo keeps its downloads in `~/.cargo`,
//! beside its tools, so its registry, git checkouts and the locks that keep
//! two cargo runs from writing them at once become links into the caches.
//! What the project's environment sets wins, and so does what the image
//! sets or keeps where a tool looks: its variables, its `~/.cargo/registry`,
//! browsers in `~/.cache/ms-playwright`.

use fctl_proto::agent::SandboxCache;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Caches the session's package managers use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ready {
    pub dir: PathBuf,
    /// The size past which the host empties them, in MiB.
    pub max_mb: Option<u64>,
    /// What points the package managers there, for the setup script and
    /// for Claude: the project's environment comes after it.
    pub env: Vec<(String, String)>,
}

/// Readies the caches in `cache` for a session whose HOME is `home`: what
/// its package managers get, or why it can't use them, for its log.
/// `image` gives the variables the image sets (fctl-agent's environment).
pub fn prepare(
    cache: &SandboxCache,
    home: &Path,
    image: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Ready, String> {
    let dir = PathBuf::from(&cache.dir);
    let keeps_own = "so this session's package managers keep their own downloads";
    match std::fs::metadata(&dir) {
        Ok(meta) if meta.is_dir() => {}
        _ => {
            return Err(format!(
                "This sandbox was made before its project shared package caches, {keeps_own}. Its next sandbox gets them."
            ));
        }
    }
    let probe = dir.join(format!(".fctl-write-test-{}", std::process::id()));
    if let Err(e) = std::fs::write(&probe, b"").and_then(|()| std::fs::remove_file(&probe)) {
        return Err(format!(
            "The package caches at {} can't be written to ({e}), {keeps_own}.",
            dir.display()
        ));
    }
    let env = env(&dir, home, apart(&dir, home), |key| image(key).is_some());
    // Claude gets no CARGO_HOME from the image, but its setup script does.
    let cargo_homes = std::iter::once(home.join(".cargo"))
        .chain(image("CARGO_HOME").map(PathBuf::from))
        .collect::<Vec<_>>();
    for cargo_home in &cargo_homes {
        if let Err(e) = link_cargo(cargo_home, &dir) {
            tracing::warn!(
                "Cargo keeps its downloads in {}: linking them to the package caches failed: {e}",
                cargo_home.display()
            );
        }
    }
    if let Err(e) = configure_pnpm(home, &dir) {
        tracing::warn!(
            "pnpm 10 and older keep their own store: writing ~/.config/pnpm/rc failed: {e}"
        );
    }
    Ok(Ready {
        dir,
        max_mb: cache.max_mb,
        env,
    })
}

/// What points the package managers at the caches in `dir`, for a session
/// whose HOME is `home`. uv hardlinks from its cache, and warns at each
/// install that it copies instead when the cache is on another disk than
/// the sandbox's (`apart`), as a mount is. `set` says what the image sets
/// already, which stays.
pub fn env(
    dir: &Path,
    home: &Path,
    apart: bool,
    set: impl Fn(&str) -> bool,
) -> Vec<(String, String)> {
    let at = |sub: &str| dir.join(sub).display().to_string();
    let mut env = vec![
        ("npm_config_cache", at("npm")),
        ("pnpm_config_store_dir", at("pnpm/store")),
        ("pnpm_config_cache_dir", at("pnpm/cache")),
        ("COREPACK_HOME", at("corepack")),
        ("YARN_CACHE_FOLDER", at("yarn/classic")),
        ("YARN_GLOBAL_FOLDER", at("yarn")),
        ("BUN_INSTALL_CACHE_DIR", at("bun")),
        ("UV_CACHE_DIR", at("uv")),
        ("PIP_CACHE_DIR", at("pip")),
        ("GOMODCACHE", at("go/mod")),
    ];
    if apart {
        env.push(("UV_LINK_MODE", "copy".into()));
    }
    // An image may come with its browsers.
    if !home.join(".cache/ms-playwright").exists() {
        env.push(("PLAYWRIGHT_BROWSERS_PATH", at("ms-playwright")));
    }
    std::iter::once(("FCTL_CACHE_DIR".to_owned(), dir.display().to_string()))
        .chain(
            env.into_iter()
                .filter(|(key, _)| !set(key))
                .map(|(key, value)| (key.to_owned(), value)),
        )
        .collect()
}

/// Whether `a` and `b` are on different filesystems.
#[cfg(unix)]
fn apart(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() != b.dev(),
        _ => true,
    }
}

#[cfg(not(unix))]
fn apart(_a: &Path, _b: &Path) -> bool {
    true
}

/// Links Cargo's downloads in `cargo_home` into the caches in `dir`: its
/// registry, its git checkouts, and the locks that keep two cargo runs from
/// writing them at once, in this sandbox or another. What is there already
/// stays, an image's own registry or the links of an earlier run.
fn link_cargo(cargo_home: &Path, dir: &Path) -> std::io::Result<()> {
    let shared = dir.join("cargo");
    std::fs::create_dir_all(shared.join("registry"))?;
    std::fs::create_dir_all(shared.join("git"))?;
    std::fs::create_dir_all(cargo_home)?;
    for name in ["registry", "git", ".package-cache", ".package-cache-mutate"] {
        let link = cargo_home.join(name);
        if std::fs::symlink_metadata(&link).is_ok() {
            continue;
        }
        symlink(&shared.join(name), &link)?;
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Points pnpm 9 and 10 at the caches in `dir` in their global
/// configuration, `~/.config/pnpm/rc`, unless it chooses a store or cache
/// of its own. pnpm 11 reads `pnpm_config_store_dir` instead.
fn configure_pnpm(home: &Path, dir: &Path) -> std::io::Result<()> {
    let rc = home.join(".config/pnpm/rc");
    let text = std::fs::read_to_string(&rc).unwrap_or_default();
    let set = |key: &str| {
        text.lines()
            .filter_map(|l| l.split_once('='))
            .any(|(k, _)| k.trim() == key)
    };
    let mut added = String::new();
    for (key, sub) in [("store-dir", "pnpm/store"), ("cache-dir", "pnpm/cache")] {
        if !set(key) {
            added.push_str(&format!("{key}={}\n", dir.join(sub).display()));
        }
    }
    if added.is_empty() {
        return Ok(());
    }
    if let Some(parent) = rc.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = text.clone();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    std::fs::write(&rc, text + &added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn package_managers_keep_their_downloads_in_the_caches() {
        let dir = Path::new("/var/cache/fctl");
        let home = Path::new("/nonexistent/home");
        let vars: BTreeMap<String, String> = env(dir, home, true, |_| false).into_iter().collect();
        assert_eq!(vars["FCTL_CACHE_DIR"], "/var/cache/fctl");
        assert_eq!(vars["npm_config_cache"], "/var/cache/fctl/npm");
        assert_eq!(vars["pnpm_config_store_dir"], "/var/cache/fctl/pnpm/store");
        assert_eq!(vars["YARN_GLOBAL_FOLDER"], "/var/cache/fctl/yarn");
        assert_eq!(vars["UV_CACHE_DIR"], "/var/cache/fctl/uv");
        assert_eq!(vars["UV_LINK_MODE"], "copy");
        assert_eq!(vars["PIP_CACHE_DIR"], "/var/cache/fctl/pip");
        assert_eq!(vars["GOMODCACHE"], "/var/cache/fctl/go/mod");
        assert_eq!(
            vars["PLAYWRIGHT_BROWSERS_PATH"],
            "/var/cache/fctl/ms-playwright"
        );
        // npm warns at every command about variables it doesn't know.
        assert!(!vars.contains_key("npm_config_store_dir"));
        // On the sandbox's own disk, uv hardlinks as it does by default;
        // what the image sets stays.
        let vars: BTreeMap<String, String> = env(dir, home, false, |key| key == "PIP_CACHE_DIR")
            .into_iter()
            .collect();
        assert!(!vars.contains_key("UV_LINK_MODE") && !vars.contains_key("PIP_CACHE_DIR"));
        assert_eq!(vars["FCTL_CACHE_DIR"], "/var/cache/fctl");
    }

    #[cfg(unix)]
    #[test]
    fn readies_caches_it_can_write_to() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let (home, dir) = (tmp.path().join("home"), tmp.path().join("cache"));
        std::fs::create_dir_all(home.join(".cache/ms-playwright"))?;
        // The image's registry stays; its git checkouts and locks link.
        std::fs::create_dir_all(home.join(".cargo/registry"))?;
        std::fs::create_dir_all(home.join(".config/pnpm"))?;
        std::fs::write(home.join(".config/pnpm/rc"), "store-dir=/opt/pnpm")?;
        let cache = SandboxCache {
            dir: dir.display().to_string(),
            max_mb: Some(10 * 1024),
        };
        let none = |_: &str| None;
        let missing = prepare(&cache, &home, &none).err().unwrap_or_default();
        assert!(
            missing.starts_with("This sandbox was made before"),
            "{missing}"
        );
        std::fs::create_dir_all(&dir)?;
        let ready = prepare(&cache, &home, &none).map_err(anyhow::Error::msg)?;
        assert_eq!(ready.max_mb, Some(10 * 1024));
        assert!(
            !ready
                .env
                .iter()
                .any(|(k, _)| k == "PLAYWRIGHT_BROWSERS_PATH")
        );
        assert!(std::fs::symlink_metadata(home.join(".cargo/registry"))?.is_dir());
        assert_eq!(
            std::fs::read_link(home.join(".cargo/git"))?,
            dir.join("cargo/git")
        );
        assert_eq!(
            std::fs::read_link(home.join(".cargo/.package-cache"))?,
            dir.join("cargo/.package-cache")
        );
        assert_eq!(
            std::fs::read_to_string(home.join(".config/pnpm/rc"))?,
            format!(
                "store-dir=/opt/pnpm\ncache-dir={}\n",
                dir.join("pnpm/cache").display()
            )
        );
        // A second run in the sandbox changes nothing.
        prepare(&cache, &home, &none).map_err(anyhow::Error::msg)?;
        let rc = std::fs::read_to_string(home.join(".config/pnpm/rc"))?;
        assert_eq!(rc.lines().count(), 2);
        assert!(std::fs::read_dir(&dir)?.flatten().all(|e| {
            !e.file_name()
                .to_string_lossy()
                .starts_with(".fctl-write-test")
        }));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn caches_it_cant_write_to_are_left_alone() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path().join("cache");
        std::fs::create_dir_all(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555))?;
        // Root writes anywhere.
        if std::fs::write(dir.join("x"), "").is_ok() {
            return Ok(());
        }
        let cache = SandboxCache {
            dir: dir.display().to_string(),
            max_mb: None,
        };
        let why = prepare(&cache, tmp.path(), &|_| None)
            .err()
            .unwrap_or_default();
        assert!(why.contains("can't be written to"), "{why}");
        assert!(!tmp.path().join(".cargo").exists());
        Ok(())
    }
}
