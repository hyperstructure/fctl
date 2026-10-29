//! Deploying an instance of the control plane to Cloudflare.
//!
//! An instance is a file in deploy/instances/<name>.toml. `deploy` renders it
//! into a wrangler config (crates/fctl-control/wrangler.deploy.toml), builds
//! the web app and the Worker, applies D1 migrations and deploys. `secrets`
//! sets what only the Worker may know, once.

use crate::util::{self, output, retry_throttled, run, run_with_stdin};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instance {
    pub worker: String,
    pub instance_name: String,
    pub public_url: String,
    pub d1: D1,
    pub github: Option<Github>,
    pub linear: Option<Linear>,
    pub email: Option<Email>,
    pub google: Option<Google>,
    /// The sign-in page lets people without an account ask for early
    /// access, and admins invite them from a list (app.fctl.sh).
    #[serde(default)]
    pub early_access: bool,
    /// Search engines may index the public pages (the sign-in page, early
    /// access): `/robots.txt` lets them and `/sitemap.xml` lists them
    /// (app.fctl.sh).
    #[serde(default)]
    pub search_engines: bool,
    /// Plain HTTP redirects to HTTPS, for every page, asset and API call
    /// (app.fctl.sh): the Worker runs first for every request and answers
    /// one over http with the redirect (`fctl_domain::https`).
    #[serde(default)]
    pub https_only: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct D1 {
    pub database_name: String,
    #[serde(default)]
    pub database_id: String,
    pub location: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Github {
    pub app_id: String,
    pub app_slug: String,
    pub client_id: Option<String>,
    pub default_owner: Option<String>,
    /// The app review agents post their reviews as where it's installed
    /// (`GITHUB_REVIEW_APP_PRIVATE_KEY` is a secret).
    pub review: Option<GithubReview>,
}

/// The GitHub App review agents post their reviews as, apart from the
/// GitHub App the instance's agents act as.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GithubReview {
    pub app_id: String,
    pub app_slug: String,
}

/// fctl's Linear app: its OAuth client ID (`LINEAR_CLIENT_SECRET` and
/// `LINEAR_WEBHOOK_SECRET` are secrets).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Linear {
    pub client_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Email {
    pub from: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Google {
    pub client_id: String,
}

fn instance_path(root: &Path, name: &str) -> PathBuf {
    root.join("deploy/instances").join(format!("{name}.toml"))
}

pub fn load(root: &Path, name: &str) -> Result<Instance> {
    let path = instance_path(root, name);
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "reading {} (copy deploy/instances/example.toml)",
            path.display()
        )
    })?;
    let inst: Instance =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    if !inst.public_url.starts_with("https://") {
        bail!("public_url must be https:// (got {})", inst.public_url);
    }
    Ok(inst)
}

impl Instance {
    /// The custom domain the Worker answers on.
    pub fn host(&self) -> &str {
        self.public_url
            .trim_start_matches("https://")
            .trim_end_matches('/')
            .split('/')
            .next()
            .unwrap_or_default()
    }

    /// The wrangler config for this instance (paths relative to crates/fctl-control).
    pub fn wrangler_toml(&self) -> String {
        let mut vars = vec![
            ("FCTL_INSTANCE_NAME", self.instance_name.clone()),
            (
                "FCTL_PUBLIC_URL",
                self.public_url.trim_end_matches('/').to_owned(),
            ),
        ];
        if let Some(g) = &self.github {
            vars.push(("GITHUB_APP_ID", g.app_id.clone()));
            vars.push(("GITHUB_APP_SLUG", g.app_slug.clone()));
            if let Some(c) = &g.client_id {
                vars.push(("GITHUB_CLIENT_ID", c.clone()));
            }
            if let Some(o) = &g.default_owner {
                vars.push(("GITHUB_DEFAULT_OWNER", o.clone()));
            }
            if let Some(r) = &g.review {
                vars.push(("GITHUB_REVIEW_APP_ID", r.app_id.clone()));
                vars.push(("GITHUB_REVIEW_APP_SLUG", r.app_slug.clone()));
            }
        }
        if let Some(l) = &self.linear {
            vars.push(("LINEAR_CLIENT_ID", l.client_id.clone()));
        }
        if let Some(e) = &self.email {
            vars.push(("FCTL_EMAIL_FROM", e.from.clone()));
        }
        if let Some(g) = &self.google {
            vars.push(("GOOGLE_CLIENT_ID", g.client_id.clone()));
        }
        if self.early_access {
            vars.push(("FCTL_EARLY_ACCESS", "1".to_owned()));
        }
        if self.search_engines {
            vars.push(("FCTL_SEARCH_ENGINES", "1".to_owned()));
        }
        if self.https_only {
            vars.push(("FCTL_HTTPS_ONLY", "1".to_owned()));
        }
        // The Worker runs first for the app's files too, to redirect them;
        // otherwise Cloudflare serves them itself, over http as well.
        let worker_first = if self.https_only {
            "true"
        } else {
            r#"["/api/*", "/robots.txt", "/sitemap.xml"]"#
        };
        let q = |s: &str| toml::Value::String(s.to_owned()).to_string();
        let vars = vars
            .iter()
            .map(|(k, v)| format!("{k} = {}", q(v)))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            r#"# Generated by `cargo xtask deploy` from deploy/instances. Do not edit.
name = {worker}
main = "build/worker/shim.mjs"
compatibility_date = "2026-10-01"
workers_dev = false
preview_urls = false

[[routes]]
pattern = {host}
custom_domain = true

[assets]
directory = "../../web/dist"
binding = "ASSETS"
not_found_handling = "single-page-application"
run_worker_first = {worker_first}

[[d1_databases]]
binding = "DB"
database_name = {db_name}
database_id = {db_id}
migrations_dir = "migrations"

[durable_objects]
bindings = [
  {{ name = "SESSION", class_name = "SessionObject" }},
  {{ name = "FLEET", class_name = "FleetObject" }},
  {{ name = "FILES", class_name = "FilesObject" }},
  {{ name = "NOTIFY", class_name = "NotifyObject" }},
  {{ name = "SCHEDULES", class_name = "ScheduleObject" }},
  {{ name = "SEARCH", class_name = "SearchObject" }},
]

[[migrations]]
tag = "v1"
new_sqlite_classes = ["SessionObject", "FleetObject"]

[[migrations]]
tag = "v2"
new_sqlite_classes = ["FilesObject"]

[[migrations]]
tag = "v3"
new_sqlite_classes = ["NotifyObject"]

[[migrations]]
tag = "v4"
new_sqlite_classes = ["ScheduleObject"]

[[migrations]]
tag = "v5"
new_sqlite_classes = ["SearchObject"]

[[ratelimits]]
name = "AUTH_LIMITER"
namespace_id = "1001"
simple = {{ limit = 30, period = 60 }}

[vars]
{vars}

# Hourly housekeeping: purges deleted sessions, drops expired sign-in records.
[triggers]
crons = ["17 * * * *"]

[observability]
enabled = true
"#,
            worker = q(&self.worker),
            host = q(self.host()),
            db_name = q(&self.d1.database_name),
            db_id = q(&self.d1.database_id),
        )
    }
}

pub struct DeployOpts {
    pub create: bool,
    pub dry_run: bool,
    pub skip_build: bool,
}

fn config_path(root: &Path) -> PathBuf {
    root.join("crates/fctl-control/wrangler.deploy.toml")
}

/// Writes the rendered wrangler config and returns its path.
pub fn render(root: &Path, inst: &Instance) -> Result<PathBuf> {
    let path = config_path(root);
    std::fs::write(&path, inst.wrangler_toml())?;
    Ok(path)
}

pub fn deploy(root: &Path, name: &str, opts: DeployOpts) -> Result<()> {
    let mut inst = load(root, name)?;
    if inst.d1.database_id.is_empty() {
        if !opts.create {
            bail!("{name} has no d1.database_id yet: run `cargo xtask deploy {name} --create`");
        }
        inst.d1.database_id = create_database(root, name, &inst)?;
    }
    let config = render(root, &inst)?;
    let control = root.join("crates/fctl-control");
    if !opts.skip_build {
        util::build_web(root)?;
        run(util::worker_build()?.arg("--release").current_dir(&control))?;
    }
    if opts.dry_run {
        eprintln!("dry run: config at {}", config.display());
        return Ok(());
    }
    retry_throttled(
        util::wrangler(root)?
            .args(["d1", "migrations", "apply", "DB", "--remote", "--config"])
            .arg(&config)
            .current_dir(&control)
            .env("CI", "1"),
        run,
    )?;
    retry_throttled(
        util::wrangler(root)?
            .args(["deploy", "--config"])
            .arg(&config)
            .current_dir(&control),
        run,
    )?;
    eprintln!("\ndeployed {} to {}", inst.worker, inst.public_url);
    let missing = missing_secrets(root, &config)?;
    if !missing.is_empty() {
        eprintln!(
            "secrets not set yet: {}. Run `cargo xtask secrets {name}`.",
            missing.join(", ")
        );
    }
    Ok(())
}

/// Creates the D1 database and records its id in the instance file.
fn create_database(root: &Path, name: &str, inst: &Instance) -> Result<String> {
    let mut cmd = util::wrangler(root)?;
    cmd.args(["d1", "create", &inst.d1.database_name]);
    if let Some(loc) = &inst.d1.location {
        cmd.args(["--location", loc]);
    }
    let out = retry_throttled(&mut cmd, output)?;
    let id = out
        .split('"')
        .skip_while(|s| *s != "database_id")
        .nth(2)
        .filter(|s| s.len() == 36)
        .map(str::to_owned)
        .with_context(|| format!("no database_id in wrangler's output:\n{out}"))?;
    let path = instance_path(root, name);
    let text = std::fs::read_to_string(&path)?;
    let updated = text.replacen("database_id = \"\"", &format!("database_id = \"{id}\""), 1);
    if updated == text {
        eprintln!(
            "add database_id = \"{id}\" under [d1] in {}",
            path.display()
        );
    } else {
        std::fs::write(&path, updated)?;
        eprintln!("recorded database_id in {}", path.display());
    }
    Ok(id)
}

fn existing_secrets(root: &Path, config: &Path) -> Result<BTreeSet<String>> {
    #[derive(Deserialize)]
    struct S {
        name: String,
    }
    let out = retry_throttled(
        util::wrangler(root)?
            .args(["secret", "list", "--format", "json", "--config"])
            .arg(config)
            .current_dir(root.join("crates/fctl-control")),
        output,
    )?;
    let list: Vec<S> =
        serde_json::from_str(out.trim()).context("parsing `wrangler secret list`")?;
    Ok(list.into_iter().map(|s| s.name).collect())
}

fn missing_secrets(root: &Path, config: &Path) -> Result<Vec<&'static str>> {
    let have = existing_secrets(root, config)?;
    Ok(["FCTL_MASTER_KEY", "FCTL_SETUP_TOKEN"]
        .into_iter()
        .filter(|s| !have.contains(*s))
        .collect())
}

pub struct SecretOpts {
    /// A GitHub App private key (.pem, PKCS#1 or PKCS#8).
    pub github_key: Option<PathBuf>,
    /// The review app's private key, likewise.
    pub github_review_key: Option<PathBuf>,
    /// Names whose values are read from stdin, one per line, in order.
    pub set: Vec<String>,
    /// Replace the setup token (prints a new setup link).
    pub new_setup_token: bool,
}

pub fn secrets(root: &Path, name: &str, opts: SecretOpts) -> Result<()> {
    let inst = load(root, name)?;
    if inst.d1.database_id.is_empty() {
        bail!("deploy {name} first: `cargo xtask deploy {name} --create`");
    }
    let config = render(root, &inst)?;
    let control = root.join("crates/fctl-control");
    let have = existing_secrets(root, &config)?;
    let put = |key: &str, value: &str| -> Result<()> {
        retry_throttled(
            util::wrangler(root)?
                .args(["secret", "put", key, "--config"])
                .arg(&config)
                .current_dir(&control),
            |cmd| run_with_stdin(cmd, value),
        )
    };
    if have.contains("FCTL_MASTER_KEY") {
        eprintln!(
            "FCTL_MASTER_KEY is set (never rotate it casually: it encrypts stored credentials)"
        );
    } else {
        put("FCTL_MASTER_KEY", &util::random_key()?)?;
    }
    if !have.contains("FCTL_SETUP_TOKEN") || opts.new_setup_token {
        let token = util::random_token()?;
        put("FCTL_SETUP_TOKEN", &token)?;
        eprintln!(
            "\nCreate the owner account (works once):\n  {}/setup#token={token}\n",
            inst.public_url.trim_end_matches('/')
        );
    }
    if let Some(pem) = &opts.github_key {
        put("GITHUB_APP_PRIVATE_KEY", &pkcs8(pem)?)?;
    }
    if let Some(pem) = &opts.github_review_key {
        put("GITHUB_REVIEW_APP_PRIVATE_KEY", &pkcs8(pem)?)?;
    }
    if !opts.set.is_empty() {
        let mut lines = std::io::stdin().lines();
        for key in &opts.set {
            let value = lines
                .next()
                .with_context(|| format!("no value on stdin for {key}"))??;
            put(key, value.trim())?;
        }
    }
    Ok(())
}

/// The key as PKCS#8 PEM (WebCrypto imports nothing else).
fn pkcs8(path: &Path) -> Result<String> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if text.contains("BEGIN PRIVATE KEY") {
        return Ok(text);
    }
    if !text.contains("BEGIN RSA PRIVATE KEY") {
        bail!("{} is not a PEM private key", path.display());
    }
    output(
        Command::new("openssl")
            .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
            .arg(path),
    )
    .context("converting the key to PKCS#8 with openssl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_an_instance() {
        let inst: Instance = toml::from_str(
            r#"
worker = "fctl"
instance_name = "Acme \"fctl\""
public_url = "https://fctl.acme.dev/"
[d1]
database_name = "fctl"
database_id = "e84e35ee-6755-4bff-a1ff-ba2539406ba0"
[github]
app_id = "1"
app_slug = "acme-fctl"
[linear]
client_id = "lin_client"
"#,
        )
        .unwrap();
        assert_eq!(inst.host(), "fctl.acme.dev");
        assert!(
            inst.wrangler_toml()
                .contains(r#"LINEAR_CLIENT_ID = "lin_client""#)
        );
        let text = inst.wrangler_toml();
        let doc: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(
            doc["vars"]["FCTL_INSTANCE_NAME"].as_str(),
            Some("Acme \"fctl\"")
        );
        assert_eq!(
            doc["vars"]["FCTL_PUBLIC_URL"].as_str(),
            Some("https://fctl.acme.dev")
        );
        assert_eq!(doc["routes"][0]["pattern"].as_str(), Some("fctl.acme.dev"));
        assert_eq!(
            doc["durable_objects"]["bindings"][2]["class_name"].as_str(),
            Some("FilesObject")
        );
        assert_eq!(
            doc["migrations"][1]["new_sqlite_classes"][0].as_str(),
            Some("FilesObject")
        );
        assert_eq!(
            doc["durable_objects"]["bindings"][3]["class_name"].as_str(),
            Some("NotifyObject")
        );
        assert_eq!(
            doc["migrations"][2]["new_sqlite_classes"][0].as_str(),
            Some("NotifyObject")
        );
        assert_eq!(
            doc["durable_objects"]["bindings"][4]["class_name"].as_str(),
            Some("ScheduleObject")
        );
        assert_eq!(
            doc["migrations"][3]["new_sqlite_classes"][0].as_str(),
            Some("ScheduleObject")
        );
        assert_eq!(
            doc["durable_objects"]["bindings"][5]["class_name"].as_str(),
            Some("SearchObject")
        );
        assert_eq!(
            doc["migrations"][4]["new_sqlite_classes"][0].as_str(),
            Some("SearchObject")
        );
        assert!(doc["vars"].get("GOOGLE_CLIENT_ID").is_none());
        assert!(doc["vars"].get("GITHUB_REVIEW_APP_ID").is_none());
        assert!(doc["vars"].get("FCTL_EARLY_ACCESS").is_none());
        assert!(doc["vars"].get("FCTL_SEARCH_ENGINES").is_none());
        assert!(doc["vars"].get("FCTL_HTTPS_ONLY").is_none());
        assert_eq!(
            doc["assets"]["run_worker_first"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|p| p.as_str())
                .collect::<Vec<_>>(),
            ["/api/*", "/robots.txt", "/sitemap.xml"]
        );
    }

    #[test]
    fn app_fctl_sh_posts_reviews_as_its_review_app() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let doc: toml::Table =
            toml::from_str(&load(&root, "fctl-sh").unwrap().wrangler_toml()).unwrap();
        assert_eq!(
            doc["vars"]["GITHUB_REVIEW_APP_ID"].as_str(),
            Some("5527664")
        );
        assert_eq!(
            doc["vars"]["GITHUB_REVIEW_APP_SLUG"].as_str(),
            Some("fctl-review")
        );
        assert_ne!(
            doc["vars"]["GITHUB_REVIEW_APP_ID"],
            doc["vars"]["GITHUB_APP_ID"]
        );
    }

    #[test]
    fn app_fctl_sh_takes_early_access_requests() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let doc: toml::Table =
            toml::from_str(&load(&root, "fctl-sh").unwrap().wrangler_toml()).unwrap();
        assert_eq!(doc["vars"]["FCTL_EARLY_ACCESS"].as_str(), Some("1"));
        let example: toml::Table =
            toml::from_str(&load(&root, "example").unwrap().wrangler_toml()).unwrap();
        assert!(example["vars"].get("FCTL_EARLY_ACCESS").is_none());
    }

    #[test]
    fn app_fctl_sh_lets_search_engines_index_its_public_pages() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let doc: toml::Table =
            toml::from_str(&load(&root, "fctl-sh").unwrap().wrangler_toml()).unwrap();
        assert_eq!(doc["vars"]["FCTL_SEARCH_ENGINES"].as_str(), Some("1"));
        let example: toml::Table =
            toml::from_str(&load(&root, "example").unwrap().wrangler_toml()).unwrap();
        assert!(example["vars"].get("FCTL_SEARCH_ENGINES").is_none());
    }

    #[test]
    fn app_fctl_sh_redirects_plain_http_to_https() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let doc: toml::Table =
            toml::from_str(&load(&root, "fctl-sh").unwrap().wrangler_toml()).unwrap();
        assert_eq!(doc["vars"]["FCTL_HTTPS_ONLY"].as_str(), Some("1"));
        assert_eq!(doc["assets"]["run_worker_first"].as_bool(), Some(true));
        let example: toml::Table =
            toml::from_str(&load(&root, "example").unwrap().wrangler_toml()).unwrap();
        assert!(example["vars"].get("FCTL_HTTPS_ONLY").is_none());
        assert!(example["assets"]["run_worker_first"].is_array());
    }
}
