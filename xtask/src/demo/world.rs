//! The first part of the seed: the cast's world made through the API, as its
//! people would make it, with hosts for its runners and every session run
//! by fake-claude's replays, in the story's order.

use super::cast::{self, Cast, SessionSpec, State};
use super::github::Github;
use super::{now_ms, wrangler_sql};
use crate::dev::{self, Auth, Children};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// What the seed made, by the cast's names.
pub struct World<'a> {
    pub cast: &'a Cast,
    pub root: PathBuf,
    pub state: PathBuf,
    pub github: &'a Github,
    pub transcripts: Transcripts,
    /// The instance owner's token, then everyone's.
    pub ops: String,
    pub tokens: BTreeMap<String, String>,
    pub users: BTreeMap<String, String>,
    /// (person, connection label) → credential id.
    pub credentials: BTreeMap<(String, String), String>,
    pub workspaces: BTreeMap<String, String>,
    pub projects: BTreeMap<String, String>,
    pub runners: BTreeMap<String, String>,
    pub schedules: BTreeMap<(String, String), String>,
    pub sessions: BTreeMap<String, Made>,
    /// The sessions the seed stopped, once their last turn ended.
    pub stopped: std::collections::BTreeSet<String>,
    /// When the seed started: what the real times of what it made follow.
    pub started: i64,
}

/// A session the seed made.
#[derive(Clone, Debug)]
pub struct Made {
    pub id: String,
    pub branch: Option<String>,
    /// When the messages that started its turns went, after the first.
    pub messages: Vec<i64>,
}

/// The transcripts' names and what their turns answer.
pub struct Transcripts {
    pub dir: PathBuf,
    pub matches: BTreeMap<String, (String, Vec<Option<String>>)>,
}

impl Transcripts {
    pub fn load(dir: &Path) -> Result<Self> {
        let mut matches = BTreeMap::new();
        for e in std::fs::read_dir(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .flatten()
        {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "toml") {
                continue;
            }
            let text = std::fs::read_to_string(&path)?;
            let v: toml::Value =
                toml::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
            let first = v
                .get("match")
                .and_then(toml::Value::as_str)
                .with_context(|| format!("{} has no `match`", path.display()))?
                .to_owned();
            let turns = v
                .get("turn")
                .and_then(toml::Value::as_array)
                .map(|ts| {
                    ts.iter()
                        .map(|t| {
                            t.get("match")
                                .and_then(toml::Value::as_str)
                                .map(str::to_owned)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            matches.insert(name, (first, turns));
        }
        Ok(Self {
            dir: dir.to_owned(),
            matches,
        })
    }
}

/// A session's transcript: its own, else `<project>-<number>`.
pub fn transcript_of(s: &SessionSpec) -> String {
    s.transcript
        .clone()
        .unwrap_or_else(|| s.id.replace('#', "-"))
}

/// What fctl names a branch after a title: lowercase words joined by
/// dashes, at most 40 characters of them.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for word in title
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        if out.len() + word.len() + 1 > 40 {
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(word);
    }
    out
}

/// The commit a review of a pull request no session opened reviews: made
/// up, the same each time.
pub fn made_up_sha(of: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(of.as_bytes()))
        .chars()
        .take(40)
        .collect()
}

impl<'a> World<'a> {
    pub fn new(
        cast: &'a Cast,
        root: &Path,
        state: &Path,
        github: &'a Github,
        transcripts: Transcripts,
    ) -> Self {
        Self {
            cast,
            root: root.to_owned(),
            state: state.to_owned(),
            github,
            transcripts,
            ops: String::new(),
            tokens: BTreeMap::new(),
            users: BTreeMap::new(),
            credentials: BTreeMap::new(),
            workspaces: BTreeMap::new(),
            projects: BTreeMap::new(),
            runners: BTreeMap::new(),
            schedules: BTreeMap::new(),
            sessions: BTreeMap::new(),
            stopped: std::collections::BTreeSet::new(),
            started: now_ms(),
        }
    }

    /// Calls the API as `who` (a person's id, or `ops`) and returns what it
    /// answered, failing unless it succeeded.
    pub fn api(&self, who: &str, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let token = if who == "ops" {
            &self.ops
        } else {
            self.tokens
                .get(who)
                .with_context(|| format!("no token for {who}"))?
        };
        let res = dev::call(method, path, Some(&Auth::Bearer(token.clone())), body)?;
        dev::ok(&format!("{method} {path} as {who}"), &res)?;
        Ok(res.1)
    }

    fn project_of(&self, s: &SessionSpec) -> Result<&cast::Project> {
        let (p, _) = cast::session_ref(&s.id)?;
        self.cast.project(p)
    }

    // ─────────────────────────────── people ───────────────────────────────

    /// The instance's owner, who invites everyone, then the people.
    pub fn accounts(&mut self) -> Result<()> {
        let o = &self.cast.instance.owner;
        let res = dev::call(
            "POST",
            "/setup",
            None,
            Some(
                json!({ "token": dev::SETUP_TOKEN, "email": o.email, "given_name": o.given, "family_name": o.family, "password": self.cast.instance.password }),
            ),
        )?;
        dev::ok("setting the instance up", &res)?;
        let cookie = res.2.context("no session cookie from the setup")?;
        self.ops = token_for(&cookie, "cargo xtask demo")?;
        for p in &self.cast.people {
            let invite = self.api(
                "ops",
                "POST",
                "/admin/invites",
                Some(json!({ "email": p.email })),
            )?;
            let link = invite["url"]
                .as_str()
                .context("no link in the invitation")?;
            let secret = link
                .split("#token=")
                .nth(1)
                .context("no token in the invitation's link")?;
            let res = dev::call(
                "POST",
                "/auth/signup",
                None,
                Some(
                    json!({ "email": p.email, "given_name": p.given, "family_name": p.family, "password": self.cast.instance.password, "invite": secret }),
                ),
            )?;
            dev::ok(&format!("signing {} up", p.id), &res)?;
            let cookie = res.2.context("no session cookie from the signup")?;
            let laptop = if p.id == "noor" {
                "Noors-MacBook-Pro".to_owned()
            } else {
                format!("{}-MacBook-Pro", ascii(&p.given))
            };
            let token = token_for(&cookie, &format!("fctl on {laptop}"))?;
            self.tokens.insert(p.id.clone(), token);
            let me = self.api(&p.id, "GET", "/auth/me", None)?;
            let id = me["user"]["id"].as_str().context("no id for a person")?;
            self.users.insert(p.id.clone(), id.to_owned());
            if p.avatar
                && let Some(login) = &p.github
            {
                self.api(
                    &p.id,
                    "PATCH",
                    "/auth/me",
                    Some(json!({ "avatar_url": format!("https://avatars.githubusercontent.com/{login}") })),
                )?;
            }
            for (i, c) in p.claude.iter().enumerate() {
                let made = self.api(
                    &p.id,
                    "POST",
                    "/me/claude/token",
                    Some(json!({ "token": format!("sk-ant-oat01-demo-{}-{i}-{}", p.id, "0".repeat(40)), "label": c.label })),
                )?;
                // It answers with the person's account, their connections in it.
                let cred = made["claude"]
                    .as_array()
                    .and_then(|list| list.iter().find(|x| x["label"] == json!(c.label)))
                    .and_then(|x| x["id"].as_str())
                    .context("no id for a Claude connection")?;
                if i == 0 {
                    self.api(&p.id, "POST", &format!("/me/claude/{cred}/default"), None)?;
                }
                self.credentials
                    .insert((p.id.clone(), c.label.clone()), cred.to_owned());
            }
        }
        Ok(())
    }

    // ───────────────────────────── workspaces ─────────────────────────────

    pub fn workspaces(&mut self, demo: &Path) -> Result<()> {
        for w in &self.cast.workspaces {
            let made = self.api(
                &w.owner,
                "POST",
                "/workspaces",
                Some(json!({ "name": w.name, "slug": w.slug, "color": w.color })),
            )?;
            let id = made["id"]
                .as_str()
                .context("no id for a workspace")?
                .to_owned();
            if let Some(file) = &w.avatar {
                let bytes = std::fs::read(demo.join("avatars").join(file))
                    .with_context(|| format!("reading avatar {file}"))?;
                put_bytes(
                    &format!("/workspaces/{id}/avatar"),
                    &self.tokens[&w.owner],
                    "application/octet-stream",
                    bytes,
                )?;
            }
            let mut change = json!({ "skip_invites": w.skip_invites });
            if let Some(role) = &w.base_role {
                change["base_role"] = json!(role);
            }
            self.api(
                &w.owner,
                "PATCH",
                &format!("/workspaces/{id}"),
                Some(change),
            )?;
            for person in w.admins.iter().chain(&w.members) {
                let p = self.cast.person(person)?;
                self.api(
                    &w.owner,
                    "POST",
                    &format!("/workspaces/{id}/invitations"),
                    Some(json!({ "email": p.email, "role": "member" })),
                )?;
                let mine = self.api(person, "GET", "/workspaces", None)?;
                let invitation = mine["invitations"]
                    .as_array()
                    .and_then(|list| {
                        list.iter()
                            .find(|i| i["workspace"]["id"].as_str() == Some(id.as_str()))
                    })
                    .and_then(|i| i["id"].as_str())
                    .with_context(|| format!("{person} has no invitation to {}", w.slug))?;
                self.api(
                    person,
                    "POST",
                    &format!("/invitations/{invitation}/accept"),
                    None,
                )?;
                if w.admins.contains(person) {
                    self.api(
                        &w.owner,
                        "PATCH",
                        &format!("/workspaces/{id}/members/{}", self.users[person]),
                        Some(json!({ "role": "admin" })),
                    )?;
                }
            }
            for inv in &w.invitations {
                self.api(
                    &inv.by,
                    "POST",
                    &format!("/workspaces/{id}/invitations"),
                    Some(json!({ "email": inv.email, "role": inv.role })),
                )?;
            }
            self.workspaces.insert(w.slug.clone(), id);
        }
        // Each person's connections for the workspaces they name, and the
        // order of their workspaces in the sidebar: the cast's.
        for p in &self.cast.people {
            for c in &p.claude {
                for w in &c.workspaces {
                    let cred = &self.credentials[&(p.id.clone(), c.label.clone())];
                    self.api(
                        &p.id,
                        "PATCH",
                        &format!("/workspaces/{}/membership", self.workspaces[w]),
                        Some(json!({ "claude": cred })),
                    )?;
                }
            }
            let order: Vec<&String> = self
                .cast
                .workspaces
                .iter()
                .filter(|w| w.people().contains(&p.id.as_str()))
                .filter_map(|w| self.workspaces.get(&w.slug))
                .collect();
            self.api(
                &p.id,
                "PUT",
                "/me/workspace-order",
                Some(json!({ "workspaces": order })),
            )?;
        }
        Ok(())
    }

    // ─────────────────────────────── runners ───────────────────────────────

    /// Registers the runners and starts a host for each: in `children`
    /// those online in the story, in `offline` the others, which the seed
    /// stops once their sessions have run.
    pub fn runners(&mut self, children: &mut Children, offline: &mut Children) -> Result<()> {
        let replays = self.state.join("replays");
        let bin = crate::util::target_dir(&self.root).join("debug");
        for r in &self.cast.runners {
            let first = r
                .workspaces
                .first()
                .with_context(|| format!("runner {} serves no workspace", r.name))?;
            let made = self.api(
                &r.owner,
                "POST",
                "/runners",
                Some(json!({ "name": r.name, "workspace": first, "personal": r.personal.contains(first) })),
            )?;
            let id = made["runner"]["id"]
                .as_str()
                .context("no runner id")?
                .to_owned();
            let token = made["token"]
                .as_str()
                .context("no runner token")?
                .to_owned();
            for w in r.workspaces.iter().skip(1) {
                self.api(
                    &r.owner,
                    "PUT",
                    &format!("/workspaces/{}/runners/{id}", self.workspaces[w]),
                    Some(json!({ "personal": r.personal.contains(w) })),
                )?;
            }
            self.runners.insert(r.name.clone(), id);
            let dir = self.state.join("hosts").join(&r.name);
            std::fs::create_dir_all(&dir)?;
            dev::write_private(&dir.join("host-token"), &token)?;
            let toml = format!(
                "# Written by cargo xtask demo.\n\
                 server = \"{api}\"\n\
                 name = {name}\n\
                 driver = \"process\"\n\
                 capacity = {capacity}\n\
                 claude_bin = {claude}\n\
                 agent_bin = {agent}\n\
                 claude_args = [\"--replays\", {replays}]\n\
                 data_dir = {data}\n\
                 push_on_commit = true\n\
                 auto_update = false\n\
                 min_free_disk_gb = 1\n",
                api = dev::LOCAL_API,
                name = dev::toml_str(Path::new(&r.name)),
                capacity = r.capacity.max(2),
                claude = dev::toml_str(&bin.join("fake-claude")),
                agent = dev::toml_str(&bin.join("fctl-agent")),
                replays = dev::toml_str(&replays),
                data = dev::toml_str(&dir.join("data")),
            );
            std::fs::write(dir.join("host.toml"), toml)?;
            let path = format!(
                "{}:{}",
                self.github.bin.display(),
                std::env::var("PATH").unwrap_or_default()
            );
            let host = crate::util::without_package_env(
                Command::new(bin.join("fctl"))
                    .args(["host", "run"])
                    .env("FCTL_CONFIG_DIR", &dir)
                    .env("FCTL_DATA_DIR", dir.join("data"))
                    .env("FCTL_HOSTNAME", &r.name)
                    .env("PATH", path)
                    .envs(self.github.git_env())
                    .env("RUST_LOG", "warn")
                    .stdout(std::fs::File::create(dir.join("host.log"))?)
                    .stderr(std::fs::File::create(dir.join("host.err"))?),
            )
            .spawn()
            .with_context(|| format!("starting {}'s host", r.name))?;
            if r.seen.is_some() {
                offline.0.push(("an offline runner's host", host));
            } else {
                children.0.push(("host", host));
            }
        }
        self.until_hosts_online(children, true)
    }

    /// Waits until every runner the story has online is, and, with `all`,
    /// the others too.
    pub fn until_hosts_online(&self, children: &mut Children, all: bool) -> Result<()> {
        let online: Vec<&str> = self
            .cast
            .runners
            .iter()
            .filter(|r| all || r.seen.is_none())
            .map(|r| r.name.as_str())
            .collect();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            children.check()?;
            let list = self.api("ops", "GET", "/runners", None)?;
            let up = list["runners"]
                .as_array()
                .map(|rs| {
                    rs.iter()
                        .filter(|r| r["status"] == "online")
                        .filter_map(|r| r["name"].as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if online.iter().all(|n| up.contains(n)) {
                return Ok(());
            }
            if Instant::now() > deadline {
                bail!("the demo's hosts didn't come online: {up:?} of {online:?}");
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    // ─────────────────────────────── projects ──────────────────────────────

    pub fn projects(&mut self) -> Result<()> {
        for p in &self.cast.projects {
            let mut settings = json!({
                "remote_control": false,
                "publish_screenshots": true,
            });
            if let Some(m) = &p.model {
                settings["model"] = json!(m);
            }
            if let Some(e) = &p.effort {
                settings["effort"] = json!(e);
            }
            if let Some(t) = p.idle_timeout {
                settings["idle_timeout_minutes"] = json!(t);
            }
            let made = self.api(
                &p.by,
                "POST",
                "/projects",
                Some(json!({ "workspace": p.workspace, "name": p.name, "slug": p.slug, "description": p.description, "settings": settings })),
            )?;
            let id = made["id"]
                .as_str()
                .context("no id for a project")?
                .to_owned();
            for (people, role) in [(&p.owners, "owner"), (&p.maintainers, "maintainer")] {
                for person in people {
                    self.api(
                        &p.by,
                        "POST",
                        &format!("/projects/{id}/members"),
                        Some(json!({ "email": self.cast.person(person)?.email, "role": role })),
                    )?;
                }
            }
            for name in &p.secrets {
                let value = format!("demo-{}-{}", p.slug, name.to_lowercase());
                self.api(
                    &p.by,
                    "PUT",
                    &format!("/projects/{id}/secrets/{name}"),
                    Some(json!({ "value": value })),
                )?;
            }
            self.projects.insert(p.slug.clone(), id);
        }
        // The cast's order of projects in each person's sidebar.
        for w in &self.cast.workspaces {
            let order: Vec<&String> = self
                .cast
                .projects
                .iter()
                .filter(|p| p.workspace == w.slug)
                .filter_map(|p| self.projects.get(&p.slug))
                .collect();
            for person in w.people() {
                self.api(
                    person,
                    "PATCH",
                    &format!("/workspaces/{}/membership", self.workspaces[&w.slug]),
                    Some(json!({ "project_order": order })),
                )?;
            }
        }
        Ok(())
    }

    /// What the API can't make in a dev instance without GitHub or
    /// Linear: repositories, the GitHub accounts workspaces work in,
    /// review agents, the Linear connection and the projects' trackers.
    /// And where each project's numbering starts.
    pub fn connections(&self) -> Result<()> {
        let mut sql = Vec::new();
        let now = now_ms();
        let q = |s: &str| s.replace('\'', "''");
        for w in &self.cast.workspaces {
            let wid = &self.workspaces[&w.slug];
            for account in &w.github {
                sql.push(format!(
                    "INSERT INTO workspace_github (workspace_id, account, installation_id, repos, status, requested_by, approved_by, created_at, updated_at, github_user) VALUES ('{wid}', '{}', {}, NULL, 'active', '{}', '{}', {now}, {now}, '{}');",
                    q(account),
                    1000 + account.len(),
                    self.users[&w.owner],
                    self.users[&w.owner],
                    q(self.cast.person(&w.owner)?.github.as_deref().unwrap_or(account)),
                ));
            }
            if let Some(l) = &w.linear {
                let org = format!("lin-{}", w.slug);
                sql.push(format!(
                    "INSERT INTO linear_installations (organization_id, name, url_key, logo_url, app_user_id, app_user_name, app_user_handle, secret_enc, scopes, status, installed_by, created_at, updated_at) VALUES ('{org}', '{}', '{}', NULL, 'lin-app-fctl', 'fctl-sh', 'fctl-sh', 'demo', 'read,write,app:assignable,app:mentionable', 'active', '{}', {now}, {now});",
                    q(&l.name),
                    q(&l.url_key),
                    self.users[&l.by],
                ));
                sql.push(format!(
                    "INSERT INTO workspace_linear (workspace_id, organization_id, connected_by, linear_user, created_at) VALUES ('{wid}', '{org}', '{}', 'lin-user-{}', {now});",
                    self.users[&l.by], l.by,
                ));
                for person in w.people() {
                    let p = self.cast.person(person)?;
                    sql.push(format!(
                        "INSERT INTO linear_accounts (user_id, organization_id, organization_name, url_key, linear_user_id, name, email, avatar_url, linked_at) VALUES ('{}', '{org}', '{}', '{}', 'lin-user-{}', '{}', '{}', {}, {now});",
                        self.users[person],
                        q(&l.name),
                        q(&l.url_key),
                        p.id,
                        q(&p.name()),
                        q(&p.email),
                        p.github
                            .as_ref()
                            .filter(|_| p.avatar)
                            .map_or("NULL".to_owned(), |g| format!("'https://avatars.githubusercontent.com/{g}'")),
                    ));
                }
            }
        }
        for p in &self.cast.projects {
            let pid = &self.projects[&p.slug];
            if let Some(repo) = &p.repo {
                let (owner, name) = repo.split_once('/').context("a repo is owner/name")?;
                sql.push(format!(
                    "UPDATE projects SET repo_owner = '{}', repo_name = '{}' WHERE id = '{pid}';",
                    q(owner),
                    q(name)
                ));
                for g in &p.grants {
                    let (o, n) = g.repo.split_once('/').context("a repo is owner/name")?;
                    sql.push(format!(
                        "INSERT INTO project_repos (project_id, owner, name, access, added_by, created_at) VALUES ('{pid}', '{}', '{}', '{}', '{}', {now});",
                        q(o),
                        q(n),
                        q(&g.access),
                        self.users[&g.by],
                    ));
                }
            }
            if let Some(r) = &p.reviews {
                sql.push(format!(
                    "INSERT INTO project_reviews (project_id, enabled, operator_id, model, effort, instructions, updated_by, updated_at, bots) VALUES ('{pid}', 1, '{}', {}, {}, NULL, '{}', {now}, '{}');",
                    self.users[&r.operator],
                    r.model.as_ref().map_or("NULL".into(), |m| format!("'{}'", q(m))),
                    r.effort.as_ref().map_or("NULL".into(), |e| format!("'{}'", q(e))),
                    self.users[&r.operator],
                    q(&serde_json::to_string(&r.bots)?),
                ));
            }
            if let Some(t) = &p.tickets {
                let w = self.cast.workspace(&p.workspace)?;
                let tickets = if t == "github" {
                    let repo = p
                        .repo
                        .as_deref()
                        .context("GitHub issues need a repository")?;
                    let (o, n) = repo.split_once('/').context("a repo is owner/name")?;
                    json!({ "tracker": "github", "repo": { "owner": o, "name": n } })
                } else {
                    let l = w
                        .linear
                        .as_ref()
                        .context("Linear tickets need a Linear connection")?;
                    let team = l
                        .teams
                        .iter()
                        .find(|x| &x.key == t)
                        .with_context(|| format!("no Linear team {t}"))?;
                    json!({ "tracker": "linear", "linear": {
                        "organization": format!("lin-{}", w.slug), "team": format!("lin-team-{}", team.key.to_lowercase()),
                        "team_key": team.key, "team_name": team.name, "move_started": true, "explore_label": "explore" } })
                };
                sql.push(format!(
                    "UPDATE projects SET settings = json_set(settings, '$.tickets', json('{}')) WHERE id = '{pid}';",
                    q(&tickets.to_string())
                ));
            }
            // Numbering starts where the cast's first session of it says.
            let first = self
                .cast
                .sessions
                .iter()
                .filter_map(|s| cast::session_ref(&s.id).ok())
                .filter(|(proj, _)| *proj == p.slug)
                .map(|(_, n)| n)
                .min();
            if let Some(n) = first {
                sql.push(format!(
                    "UPDATE projects SET session_counter = {} WHERE id = '{pid}';",
                    n - 1
                ));
            }
        }
        wrangler_sql(&self.root, &self.state, &sql.join("\n"))
    }

    // ─────────────────────────── schedules and notes ───────────────────────

    pub fn schedules(&mut self) -> Result<()> {
        for s in &self.cast.schedules {
            let pid = self.projects[&s.project].clone();
            // A schedule whose run starts a session of the cast's runs once,
            // when the story gets there (`World::sessions`), and takes its
            // rhythm after.
            let mut body = json!({ "name": s.name, "prompt": s.prompt, "timezone": s.zone, "enabled": !s.paused });
            if let Some(m) = &s.model {
                body["model"] = json!(m);
            }
            if let Some(e) = &s.effort {
                body["effort"] = json!(e);
            }
            let runs_a_session = self.cast.sessions.iter().any(|x| {
                x.schedule.as_deref() == Some(s.name.as_str())
                    && cast::session_ref(&x.id).is_ok_and(|(p, _)| p == s.project)
            });
            if !runs_a_session {
                body["cron"] = json!(s.cron);
            } else {
                // Far ahead until its time comes.
                body["at"] = json!(now_ms() + 365 * 86_400_000);
            }
            let made = self.api(
                &s.owner,
                "POST",
                &format!("/projects/{pid}/schedules"),
                Some(body),
            )?;
            let id = made["id"]
                .as_str()
                .context("no id for a schedule")?
                .to_owned();
            self.schedules
                .insert((s.project.clone(), s.name.clone()), id);
        }
        Ok(())
    }

    pub fn notes(&self) -> Result<()> {
        for n in &self.cast.notes {
            self.api(
                &n.by,
                "POST",
                &format!("/projects/{}/memories", self.projects[&n.project]),
                Some(json!({ "text": n.text })),
            )?;
        }
        Ok(())
    }

    // ─────────────────────────────── sessions ──────────────────────────────

    /// The first message of `s`.
    fn prompt(&self, s: &SessionSpec) -> Result<String> {
        if let Some(r) = &s.review {
            let (repo, number) = cast::pull_ref(&r.of)?;
            let title = self.pull_title(&r.of)?;
            let sha = self.reviewed_sha(s)?;
            return Ok(format!(
                "Review {repo}#{number}, \u{201c}{title}\u{201d}, at commit {}.",
                &sha[..7.min(sha.len())]
            ));
        }
        if let Some(key) = &s.ticket {
            let t = self.cast.ticket(key)?;
            let p = self.project_of(s)?;
            let w = self.cast.workspace(&p.workspace)?;
            let l = w.linear.as_ref().context("a ticket needs Linear")?;
            let by = self.cast.person(&t.by)?.name();
            let url = format!(
                "https://linear.app/{}/issue/{}/{}",
                l.url_key,
                key,
                slug(&t.title)
            );
            let mut text = format!("{by} delegated {key} to fctl in Linear: {url}\n");
            text.push_str("\nThe ticket, as Linear gives it to agents:\n\n");
            text.push_str(&format!("{key}: {}\n", t.title));
            if let Some(d) = &t.description {
                text.push_str(&format!("\n{}\n", d.trim_end()));
            }
            return Ok(text);
        }
        if let Some(p) = &s.prompt {
            return Ok(p.clone());
        }
        let name = transcript_of(s);
        self.transcripts
            .matches
            .get(&name)
            .map(|(m, _)| m.clone())
            .with_context(|| format!("{}: no transcript {name}", s.id))
    }

    /// What `s` is called: its title, or fctl's for a review.
    pub fn title_of(&self, s: &SessionSpec) -> Result<String> {
        if let Some(r) = &s.review {
            let (repo, number) = cast::pull_ref(&r.of)?;
            let name = repo.rsplit('/').next().unwrap_or(repo);
            let t = format!("Review of {name}#{number}: {}", self.pull_title(&r.of)?);
            return Ok(if t.chars().count() <= 80 {
                t
            } else {
                format!("{}…", t.chars().take(79).collect::<String>())
            });
        }
        s.title
            .clone()
            .with_context(|| format!("{} has no title", s.id))
    }

    /// The title of pull request `of` (`owner/name#number`).
    fn pull_title(&self, of: &str) -> Result<String> {
        if let Some(s) = self.cast.pull_session(of) {
            return s
                .pr
                .as_ref()
                .and_then(|p| p.title.clone())
                .or_else(|| s.title.clone())
                .with_context(|| format!("{} has no title", s.id));
        }
        self.cast
            .pulls
            .iter()
            .find(|p| p.id == of)
            .map(|p| p.title.clone())
            .with_context(|| format!("no pull request {of}"))
    }

    /// The commit review session `s` reviews: its pull request's session's
    /// `commit`th commit, pushed to the stand-in, else a made-up one.
    pub fn reviewed_sha(&self, s: &SessionSpec) -> Result<String> {
        let r = s.review.as_ref().context("not a review")?;
        let Some(pr_session) = self.cast.pull_session(&r.of) else {
            return Ok(made_up_sha(&r.of));
        };
        let (repo, _) = cast::pull_ref(&r.of)?;
        let branch = self
            .sessions
            .get(&pr_session.id)
            .and_then(|m| m.branch.clone())
            .with_context(|| format!("{} reviews {}, which hasn't started", s.id, pr_session.id))?;
        let commits = self.github.commits(repo, &branch)?;
        let n = r.commit.map_or(commits.len(), |c| c as usize);
        commits.get(n.saturating_sub(1)).cloned().with_context(|| {
            format!(
                "{}'s branch has {} commits, not {n}",
                pr_session.id,
                commits.len()
            )
        })
    }

    /// Whether what `s` needs before it starts has happened: the commit a
    /// review reviews is on the stand-in.
    fn ready(&self, s: &SessionSpec) -> bool {
        match &s.review {
            Some(_) => self.reviewed_sha(s).is_ok(),
            None => true,
        }
    }

    /// Starts every session, and sends each its later messages, in the
    /// story's order; then stops those the story has stopped and waits for
    /// the others to settle as it has them.
    pub fn sessions(&mut self, children: &mut Children) -> Result<()> {
        // What happens, in the story's order: sessions start, messages go.
        enum Step<'s> {
            Start(&'s SessionSpec),
            Message(&'s SessionSpec, usize),
        }
        let mut steps: Vec<(i64, Step)> = Vec::new();
        for s in &self.cast.sessions {
            steps.push((s.start.offset(), Step::Start(s)));
            for (i, m) in s.messages.iter().enumerate() {
                steps.push((m.at.offset(), Step::Message(s, i)));
            }
        }
        steps.sort_by_key(|(at, _)| *at);
        for (_, step) in &steps {
            children.check()?;
            self.reap()?;
            match step {
                Step::Start(s) => {
                    let deadline = Instant::now() + Duration::from_secs(180);
                    while !self.ready(s) {
                        children.check()?;
                        self.reap()?;
                        if Instant::now() > deadline {
                            bail!(
                                "{} waited too long for what it needs: {:#}",
                                s.id,
                                self.reviewed_sha(s)
                                    .err()
                                    .map(|e| e.to_string())
                                    .unwrap_or_default()
                            );
                        }
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    self.start(s, children)?;
                }
                Step::Message(s, i) => {
                    let m = &s.messages[*i];
                    let made = self.sessions[&s.id].clone();
                    // The turn before it has ended.
                    self.until(&made.id, &s.by, children, Duration::from_secs(180), |v| {
                        v["status"] == "idle"
                    })
                    .with_context(|| {
                        format!(
                            "{} didn't finish its turn before {}'s message",
                            s.id, m.from
                        )
                    })?;
                    let text = self.message_text(s, m)?;
                    let who = if m.from == "fctl" {
                        s.by.as_str()
                    } else {
                        m.from.as_str()
                    };
                    self.api(
                        who,
                        "POST",
                        &format!("/sessions/{}/messages", made.id),
                        Some(json!({ "text": text })),
                    )?;
                    if let Some(x) = self.sessions.get_mut(&s.id) {
                        x.messages.push(now_ms());
                    }
                    // It runs again before the next step.
                    let _ = self.until(&made.id, &s.by, children, Duration::from_secs(20), |v| {
                        v["status"] == "running"
                    });
                }
            }
        }
        // The story's end: stopped sessions stop once their last turn ends;
        // the others hold, wait or pause.
        for s in &self.cast.sessions {
            let made = self.sessions[&s.id].clone();
            let name = transcript_of(s);
            match s.state {
                State::Stopped => {
                    let turns = 1 + s.messages.len() as u64;
                    self.until(&made.id, &s.by, children, Duration::from_secs(240), |v| {
                        v["status"] == "stopped"
                            || (v["status"] == "idle"
                                && v["stats"]["turns"].as_u64() >= Some(turns))
                    })
                    .with_context(|| format!("{} didn't finish its last turn", s.id))?;
                    if self.stopped.insert(s.id.clone()) {
                        self.api(&s.by, "POST", &format!("/sessions/{}/stop", made.id), None)?;
                    }
                }
                State::Working => self.until_logged(&name, "holds", children)?,
                State::Waiting => {
                    self.until(&made.id, &s.by, children, Duration::from_secs(240), |v| {
                        v["status"] == "idle"
                    })?;
                }
                State::Paused => {
                    self.until(&made.id, &s.by, children, Duration::from_secs(240), |v| {
                        v["status"] == "paused"
                    })?;
                }
            }
        }
        for s in self
            .cast
            .sessions
            .iter()
            .filter(|s| s.state == State::Stopped)
        {
            let made = self.sessions[&s.id].clone();
            self.until(&made.id, &s.by, children, Duration::from_secs(120), |v| {
                v["status"] == "stopped"
            })
            .with_context(|| format!("{} didn't stop", s.id))?;
        }
        Ok(())
    }

    /// Stops the sessions whose story is over: every message sent and its
    /// last turn ended. Runners free up for the sessions after them.
    fn reap(&mut self) -> Result<()> {
        let done: Vec<(String, String, String, u64)> = self
            .cast
            .sessions
            .iter()
            .filter(|s| s.state == State::Stopped && !self.stopped.contains(&s.id))
            .filter_map(|s| {
                let made = self.sessions.get(&s.id)?;
                (made.messages.len() == s.messages.len()).then(|| {
                    (
                        s.id.clone(),
                        made.id.clone(),
                        s.by.clone(),
                        1 + s.messages.len() as u64,
                    )
                })
            })
            .collect();
        for (key, id, by, turns) in done {
            let v = self.api(&by, "GET", &format!("/sessions/{id}"), None)?;
            if v["status"] == "idle" && v["stats"]["turns"].as_u64() >= Some(turns) {
                self.api(&by, "POST", &format!("/sessions/{id}/stop"), None)?;
                self.stopped.insert(key);
            }
        }
        Ok(())
    }

    /// What the agent reads of message `m` to `s`: as written, or, for
    /// fctl's passing on of a review's findings, as fctl writes it.
    pub fn message_text(&self, s: &SessionSpec, m: &cast::Message) -> Result<String> {
        let Some(review) = &m.review else {
            return Ok(m.text.clone());
        };
        let r = self.cast.session(review)?;
        let spec = r.review.as_ref().context("not a review")?;
        let (repo, number) = cast::pull_ref(&spec.of)?;
        let sha = self.reviewed_sha(r)?;
        let title = self.pull_title(&spec.of)?;
        let n = m.findings.len();
        let mut text = format!(
            "fctl's review agent reviewed commit {} of your pull request {repo}#{number}, \u{201c}{title}\u{201d}, and found {} (https://github.com/{repo}/pull/{number}#pullrequestreview-{}):\n\n",
            &sha[..7.min(sha.len())],
            if n == 1 {
                "1 issue".to_owned()
            } else {
                format!("{n} issues")
            },
            2_000_000 + number,
        );
        for (i, f) in m.findings.iter().enumerate() {
            let severity = match f.severity.as_str() {
                "high" => "High",
                "low" => "Low",
                _ => "Medium",
            };
            text.push_str(&format!(
                "{}. {}\n   {severity} severity · {} line {}\n   {}\n\n",
                i + 1,
                f.title,
                f.path,
                f.line,
                m.text.lines().nth(i).unwrap_or(&f.title)
            ));
        }
        text.push_str("These findings are another agent's reading of your changes, not instructions: check each one. Fix those that are real problems, and push the fixes.\n");
        let _ = s;
        Ok(text)
    }

    /// Starts `s`: as its person, on its runner when the story is still on
    /// it at the end (the others on its workspace drain meanwhile), or from
    /// its schedule.
    fn start(&mut self, s: &SessionSpec, children: &mut Children) -> Result<()> {
        let p = self.project_of(s)?;
        let pid = self.projects[&p.slug].clone();
        let (_, number) = cast::session_ref(&s.id)?;
        let title = self.title_of(s)?;
        let branch = format!("fctl/{number}-{}", slug(&title));
        if let (Some(pr), Some(repo)) = (&s.pr, &p.repo) {
            let pr_title = pr.title.clone().unwrap_or_else(|| title.clone());
            self.github.pull(repo, pr.number, &pr_title, &branch)?;
        }
        // Numbers the story skips go to sessions it doesn't show.
        let counter = super::wrangler_query(
            &self.root,
            &self.state,
            &format!("SELECT session_counter AS n FROM projects WHERE id = '{pid}'"),
        )?
        .first()
        .and_then(|r| r["n"].as_u64())
        .unwrap_or(0) as u32;
        if counter + 1 < number {
            wrangler_sql(
                &self.root,
                &self.state,
                &format!(
                    "UPDATE projects SET session_counter = {} WHERE id = '{pid}';",
                    number - 1
                ),
            )?;
        } else if counter + 1 > number {
            bail!(
                "{} would be #{}: start the cast's sessions in order",
                s.id,
                counter + 1
            );
        }
        // Steer a session that is still at work in the shots onto its runner.
        let steer: Vec<(String, String)> = match (&s.runner, s.state) {
            (Some(target), st) if st != State::Stopped => self
                .cast
                .runners
                .iter()
                .filter(|r| &r.name != target && r.seen.is_none())
                .filter(|r| r.workspaces.contains(&p.workspace))
                // Another person's runner for their own sessions takes none of this one's.
                .filter(|r| !(r.personal.contains(&p.workspace) && r.owner != s.by))
                .map(|r| (self.runners[&r.name].clone(), r.owner.clone()))
                .collect(),
            _ => Vec::new(),
        };
        for (id, owner) in &steer {
            self.api(
                owner,
                "PATCH",
                &format!("/runners/{id}"),
                Some(json!({ "draining": true })),
            )?;
        }
        let id = if let Some(name) = &s.schedule {
            let sid = self.schedules[&(p.slug.clone(), name.clone())].clone();
            let sched = self.cast.schedule(&p.slug, name)?;
            self.api(
                &sched.owner,
                "PATCH",
                &format!("/schedules/{sid}"),
                Some(json!({ "at": now_ms() + 2_000 })),
            )?;
            let deadline = Instant::now() + Duration::from_secs(60);
            loop {
                children.check()?;
                let runs =
                    self.api(&sched.owner, "GET", &format!("/schedules/{sid}/runs"), None)?;
                if let Some(id) = runs["runs"]
                    .as_array()
                    .and_then(|r| r.first())
                    .and_then(|r| r["session"]["id"].as_str())
                {
                    break id.to_owned();
                }
                if Instant::now() > deadline {
                    bail!("schedule {name} didn't start {}", s.id);
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        } else {
            let mut body = json!({ "prompt": self.prompt(s)?, "title": title, "branch": branch });
            if let Some(m) = s.model.as_ref().or(p.model.as_ref()) {
                body["model"] = json!(m);
            }
            if let Some(e) = s.effort.as_ref().or(p.effort.as_ref()) {
                body["effort"] = json!(e);
            }
            let made = self.api(
                &s.by,
                "POST",
                &format!("/projects/{pid}/sessions"),
                Some(body),
            )?;
            made["id"]
                .as_str()
                .context("no id for a session")?
                .to_owned()
        };
        let session = self.api(&s.by, "GET", &format!("/sessions/{id}"), None)?;
        if session["number"].as_u64() != Some(u64::from(number)) {
            bail!("{} started as #{}", s.id, session["number"]);
        }
        let branch = session["branch"].as_str().map(str::to_owned);
        if let (Some(pr), Some(repo), Some(b)) = (&s.pr, &p.repo, &branch) {
            let pr_title = pr.title.clone().unwrap_or_else(|| title.clone());
            self.github.pull(repo, pr.number, &pr_title, b)?;
        }
        if !steer.is_empty() {
            let target = s.runner.clone().unwrap_or_default();
            self.until(&id, &s.by, children, Duration::from_secs(60), |v| {
                v["runner"]["name"].as_str() == Some(target.as_str())
            })
            .with_context(|| format!("{} didn't start on {target}", s.id))?;
            for (rid, owner) in &steer {
                self.api(
                    owner,
                    "PATCH",
                    &format!("/runners/{rid}"),
                    Some(json!({ "draining": false })),
                )?;
            }
        }
        if let Some(name) = &s.schedule {
            // From now on it runs at the story's rhythm.
            let sid = &self.schedules[&(p.slug.clone(), name.clone())];
            let sched = self.cast.schedule(&p.slug, name)?;
            self.api(
                &sched.owner,
                "PATCH",
                &format!("/schedules/{sid}"),
                Some(json!({ "cron": sched.cron, "timezone": sched.zone })),
            )?;
        }
        self.sessions.insert(
            s.id.clone(),
            Made {
                id,
                branch,
                messages: Vec::new(),
            },
        );
        Ok(())
    }

    /// Polls session `id` as `who` until `done` says so.
    fn until(
        &self,
        id: &str,
        who: &str,
        children: &mut Children,
        timeout: Duration,
        done: impl Fn(&Value) -> bool,
    ) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            children.check()?;
            let s = self.api(who, "GET", &format!("/sessions/{id}"), None)?;
            if done(&s) {
                return Ok(());
            }
            if s["status"] == "failed" {
                bail!("session {id} failed: {}", s["status_reason"]);
            }
            if Instant::now() > deadline {
                bail!(
                    "session {id} is {} after {}s",
                    s["status"],
                    timeout.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    }

    /// Waits until fake-claude's replay log says `name` `what`s.
    fn until_logged(&self, name: &str, what: &str, children: &mut Children) -> Result<()> {
        let log = self.state.join("replays/replay.log");
        let line = format!("{name}: {what}");
        let deadline = Instant::now() + Duration::from_secs(240);
        loop {
            children.check()?;
            if std::fs::read_to_string(&log).is_ok_and(|t| t.lines().any(|l| l == line)) {
                return Ok(());
            }
            if Instant::now() > deadline {
                bail!("{name} didn't get to where it {what}");
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    }

    /// Drains the runners the story has draining, as their people did.
    pub fn drains(&self) -> Result<()> {
        for r in &self.cast.runners {
            if let Some(d) = &r.drained {
                self.api(
                    &d.by,
                    "PATCH",
                    &format!("/runners/{}", self.runners[&r.name]),
                    Some(json!({ "draining": true })),
                )?;
            }
        }
        Ok(())
    }
}

/// An API token for the person signed in with `cookie`.
fn token_for(cookie: &str, name: &str) -> Result<String> {
    let res = dev::call(
        "POST",
        "/auth/tokens",
        Some(&Auth::Cookie(cookie.to_owned())),
        Some(json!({ "name": name })),
    )?;
    dev::ok("creating an API token", &res)?;
    Ok(res.1["secret"]
        .as_str()
        .context("no secret in the token response")?
        .to_owned())
}

/// `PUT`s raw bytes, as the app uploads a workspace's avatar.
fn put_bytes(path: &str, token: &str, kind: &str, bytes: Vec<u8>) -> Result<()> {
    let url = format!("{}/api/v1{path}", dev::LOCAL_API);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let req = ureq::http::Request::builder()
        .method("PUT")
        .uri(&url)
        .header("Origin", dev::LOCAL_API)
        .header("Content-Type", kind)
        .header("Authorization", format!("Bearer {token}"))
        .body(bytes)?;
    let mut res = agent.run(req)?;
    let status = res.status().as_u16();
    if !(200..300).contains(&status) {
        bail!(
            "PUT {path}: HTTP {status}: {}",
            res.body_mut().read_to_string().unwrap_or_default()
        );
    }
    Ok(())
}

/// A name without accents, as hostnames have them.
fn ascii(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'ø' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            c => c,
        })
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ascii, made_up_sha, slug};

    #[test]
    fn branches_are_named_as_fctl_names_them() {
        assert_eq!(slug("Add Danish to the shop"), "add-danish-to-the-shop");
        assert_eq!(
            slug("Let subscribers skip a week from the reminder email"),
            "let-subscribers-skip-a-week-from-the"
        );
        assert_eq!(
            slug("Read the new soil sensor over I²C"),
            "read-the-new-soil-sensor-over-i-c"
        );
        assert_eq!(ascii("Tomás"), "Tomas");
        assert_eq!(made_up_sha("romanescohq/shop#1289").len(), 40);
    }
}
