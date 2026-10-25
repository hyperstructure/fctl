//! The demo cast as `xtask/demo/cast.toml` writes it down: who and what
//! exists, and the sessions they ran, with the times of the story.
//!
//! Times are as the story tells them, from the moment of the shots, a
//! Tuesday at 09:10 ([`When`]); the seed turns them into offsets from
//! whenever it runs.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cast {
    pub instance: Instance,
    #[serde(rename = "person")]
    pub people: Vec<Person>,
    #[serde(rename = "workspace")]
    pub workspaces: Vec<Workspace>,
    #[serde(rename = "runner")]
    pub runners: Vec<RunnerSpec>,
    #[serde(rename = "repo", default)]
    pub repos: Vec<Repo>,
    #[serde(rename = "project")]
    pub projects: Vec<Project>,
    #[serde(rename = "pull", default)]
    pub pulls: Vec<Pull>,
    #[serde(rename = "schedule", default)]
    pub schedules: Vec<Schedule>,
    #[serde(rename = "note", default)]
    pub notes: Vec<Note>,
    #[serde(rename = "ticket", default)]
    pub tickets: Vec<Ticket>,
    #[serde(rename = "session")]
    pub sessions: Vec<SessionSpec>,
}

/// The instance the cast lives on.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instance {
    /// What the app calls itself.
    pub name: String,
    /// The account that sets the instance up and invites everyone: no
    /// workspace of the cast's has it.
    pub owner: Account,
    /// Everyone's password.
    pub password: String,
    /// The GitHub App's slug, which agents' pull requests name as their
    /// author (`<slug>[bot]`).
    pub github_app: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub email: String,
    pub given: String,
    pub family: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Person {
    pub id: String,
    pub given: String,
    pub family: String,
    pub email: String,
    pub github: Option<String>,
    /// Their avatar is `<github>.png` in the avatars.
    #[serde(default)]
    pub avatar: bool,
    pub joined: When,
    /// Their Claude connections, the default first.
    #[serde(default)]
    pub claude: Vec<Connection>,
}

impl Person {
    pub fn name(&self) -> String {
        format!("{} {}", self.given, self.family)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub label: String,
    /// `max` or `pro`.
    pub plan: String,
    /// The workspaces whose sessions run on it, when it isn't the default.
    #[serde(default)]
    pub workspaces: Vec<String>,
    pub usage: Option<Usage>,
}

/// Plan usage as the app shows it.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub five_hour: f64,
    pub seven_day: f64,
    /// The five-hour window is at its limit, until this long from now.
    pub limited: Option<Dur>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub slug: String,
    pub name: String,
    /// `yellow`, `pink`, `blue` or `ink`: its initials' color without an avatar.
    pub color: String,
    /// Its avatar, a file in the avatars.
    pub avatar: Option<String>,
    pub owner: String,
    #[serde(default)]
    pub admins: Vec<String>,
    #[serde(default)]
    pub members: Vec<String>,
    /// When each of them joined, where it isn't when their account did.
    #[serde(default)]
    pub joined: BTreeMap<String, When>,
    pub base_role: Option<String>,
    #[serde(default)]
    pub skip_invites: bool,
    pub created: When,
    /// The GitHub accounts it works in.
    #[serde(default)]
    pub github: Vec<String>,
    pub linear: Option<Linear>,
    #[serde(rename = "invitation", default)]
    pub invitations: Vec<Invitation>,
}

impl Workspace {
    /// Everyone in it, the owner first.
    pub fn people(&self) -> Vec<&str> {
        std::iter::once(self.owner.as_str())
            .chain(self.admins.iter().map(String::as_str))
            .chain(self.members.iter().map(String::as_str))
            .collect()
    }
}

/// The Linear workspace a workspace connected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Linear {
    pub name: String,
    pub url_key: String,
    pub by: String,
    pub at: When,
    #[serde(rename = "team", default)]
    pub teams: Vec<Team>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Team {
    pub key: String,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    pub email: String,
    #[serde(default = "member")]
    pub role: String,
    pub by: String,
    pub at: When,
}

fn member() -> String {
    "member".to_owned()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerSpec {
    pub name: String,
    pub owner: String,
    pub os: String,
    pub arch: String,
    pub cpus: u32,
    pub memory_gb: u64,
    pub driver: String,
    pub capacity: u32,
    #[serde(default)]
    pub workspaces: Vec<String>,
    /// Of `workspaces`, where it runs only its owner's sessions.
    #[serde(default)]
    pub personal: Vec<String>,
    pub created: When,
    /// Offline, last seen then.
    pub seen: Option<When>,
    pub drained: Option<Drained>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drained {
    pub at: When,
    pub by: String,
}

/// A repository on the stand-in GitHub, and what it holds at first: the
/// files under `xtask/demo/repos/<name>`, when there are any.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repo {
    pub name: String,
    pub created: When,
    /// Who its first commit names.
    pub by: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub slug: String,
    pub workspace: String,
    pub name: String,
    pub description: String,
    pub by: String,
    pub created: When,
    pub repo: Option<String>,
    #[serde(rename = "grant", default)]
    pub grants: Vec<Grant>,
    #[serde(default)]
    pub owners: Vec<String>,
    #[serde(default)]
    pub maintainers: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Minutes an idle session waits before it stops.
    pub idle_timeout: Option<u32>,
    pub reviews: Option<Reviews>,
    /// The Linear team whose tickets it takes, or `github` for the issues
    /// of its repositories.
    pub tickets: Option<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
    /// How many sessions its header counts: older ones than the cast's
    /// are made up to come to it, titled from `history`.
    pub counted: Option<u32>,
    /// What its older sessions were about, oldest first: the newest of
    /// them take these titles, and older ones again from the end.
    #[serde(default)]
    pub history: Vec<String>,
    /// How many retrospectives its tab counts, completed, partial and
    /// blocked: older sessions' make up what the cast's don't.
    #[serde(default)]
    pub retros: Vec<u32>,
}

/// Another repository a project's agents may use.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub repo: String,
    pub access: String,
    pub by: String,
    pub at: When,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reviews {
    pub operator: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    #[serde(default)]
    pub bots: Vec<String>,
    pub since: When,
}

/// A pull request no session of the cast's opened: a bot's, or an outside
/// contributor's.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pull {
    /// `owner/name#number`.
    pub id: String,
    pub title: String,
    pub author: String,
    /// `open` or `merged`.
    pub state: String,
    /// When it was opened.
    pub at: When,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    pub project: String,
    pub name: String,
    pub cron: Option<String>,
    pub zone: String,
    pub owner: String,
    pub prompt: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    #[serde(default)]
    pub paused: bool,
    pub created: When,
    /// Its latest run, when it didn't start a session of the cast's
    /// (those name it themselves): skipped, and why.
    pub skipped: Option<Skipped>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skipped {
    pub at: When,
    pub reason: String,
}

/// A note a person wrote in a project's memory.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub project: String,
    pub by: String,
    pub at: When,
    pub text: String,
    /// It went stale: the session (`project#number`) whose agent replaced
    /// it with a note of its own, or the reason someone gave.
    pub replaced_by: Option<String>,
    pub stale: Option<String>,
    /// The session (`project#number`) whose agent marked it stale.
    pub stale_by: Option<String>,
}

/// A Linear ticket someone delegated to fctl.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ticket {
    pub key: String,
    pub title: String,
    pub description: Option<String>,
    pub by: String,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSpec {
    /// `project#number`.
    pub id: String,
    pub by: String,
    pub start: When,
    /// How long it worked, to its last turn's end: a stopped session's,
    /// and one waiting or paused when it says; absent while it works.
    pub took: Option<Dur>,
    #[serde(default)]
    pub state: State,
    pub runner: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// What its agent names it; a review's is fctl's.
    pub title: Option<String>,
    /// Its first message: absent, the transcript's `match`.
    pub prompt: Option<String>,
    pub transcript: Option<String>,
    pub pr: Option<SessionPull>,
    /// The ticket it started from.
    pub ticket: Option<String>,
    /// The schedule it started from.
    pub schedule: Option<String>,
    pub review: Option<ReviewSpec>,
    #[serde(rename = "message", default)]
    pub messages: Vec<Message>,
    /// People watching its page in the shots.
    #[serde(default)]
    pub watching: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    #[default]
    Stopped,
    /// At work: its transcript holds a call.
    Working,
    /// Waiting for a message.
    Waiting,
    /// At its Claude plan's limit.
    Paused,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionPull {
    pub number: u64,
    /// `open`, `draft`, `merged` or `closed`.
    pub state: String,
    /// When it merged or closed.
    pub at: Option<When>,
    pub title: Option<String>,
    pub passed: Option<u32>,
    pub failed: Option<u32>,
    pub pending: Option<u32>,
}

/// What a review agent's session reviewed, and what came of it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewSpec {
    /// The pull request: `owner/name#number`.
    pub of: String,
    /// Which of its session's commits: the first is 1. Absent: its newest
    /// one, or a made-up one for a pull request no session opened.
    pub commit: Option<u32>,
    /// `running`, `passed`, `commented` or `superseded`.
    pub result: String,
    #[serde(default)]
    pub findings: u32,
    #[serde(default)]
    pub resolved: u32,
    #[serde(default)]
    pub still_open: u32,
    pub took: Option<Dur>,
}

/// A later message to a session's agent.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub at: When,
    /// A person, or `fctl` for fctl's own messages.
    pub from: String,
    /// What the agent reads.
    pub text: String,
    /// fctl passing on a review's findings: the review's session
    /// (`project#number`), with each finding.
    pub review: Option<String>,
    #[serde(rename = "finding", default)]
    pub findings: Vec<Finding>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub severity: String,
    pub title: String,
    pub path: String,
    pub line: u32,
}

/// A moment of the story, as it tells it: `now`, `-18m`, `-3w`, `-14mo`,
/// `today 08:52`, `yesterday 22:40`, `saturday 03:00`, `last thursday
/// 14:20`, all from the moment of the shots, a Tuesday at 09:10.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(try_from = "String")]
pub struct When(pub String);

impl TryFrom<String> for When {
    type Error = String;
    fn try_from(s: String) -> std::result::Result<Self, String> {
        offset_of(&s)?;
        Ok(Self(s))
    }
}

impl When {
    /// Seconds from the moment of the shots: negative before it.
    pub fn offset(&self) -> i64 {
        offset_of(&self.0).unwrap_or_default()
    }
}

/// The moment of the shots, as the story has it: a Tuesday, at 09:10.
const SHOT_MINUTE: i64 = 9 * 60 + 10;

fn offset_of(s: &str) -> std::result::Result<i64, String> {
    let t = s.trim().to_lowercase();
    let bad = || {
        format!(
            "{s:?} isn't a time of the story: say now, -18m, -3w, -14mo, today 08:52, yesterday 22:40, saturday 03:00 or last thursday 14:20"
        )
    };
    if t == "now" {
        return Ok(0);
    }
    if let Some(ago) = t.strip_prefix('-') {
        return Ok(-dur_of(ago).map_err(|_| bad())?);
    }
    let (day, clock) = t.rsplit_once(' ').ok_or_else(bad)?;
    let day = day.trim().strip_prefix("last ").unwrap_or(day.trim());
    // Days before the Tuesday of the shots.
    let back = match day {
        "today" => 0,
        "yesterday" | "monday" => 1,
        "sunday" => 2,
        "saturday" => 3,
        "friday" => 4,
        "thursday" => 5,
        "wednesday" => 6,
        "tuesday" => 7,
        _ => return Err(bad()),
    };
    let (h, m) = clock.split_once(':').ok_or_else(bad)?;
    let (h, m): (i64, i64) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return Err(bad());
    }
    Ok((h * 60 + m - SHOT_MINUTE) * 60 - back * 86_400)
}

/// A length of time: `90s`, `51m`, `1m31s`, `2h`, `3d`, `3w`, `14mo`, `3y`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(try_from = "String")]
pub struct Dur(pub i64);

impl TryFrom<String> for Dur {
    type Error = String;
    fn try_from(s: String) -> std::result::Result<Self, String> {
        dur_of(&s).map(Self)
    }
}

/// Seconds in a length of time.
fn dur_of(s: &str) -> std::result::Result<i64, String> {
    let bad =
        || format!("{s:?} isn't a length of time: say 90s, 51m, 1m31s, 2h, 3d, 3w, 14mo or 3y");
    let t = s.trim();
    let mut total = 0i64;
    let mut rest = t;
    while !rest.is_empty() {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 {
            return Err(bad());
        }
        let (n, after) = rest.split_at(digits);
        let n: i64 = n.parse().map_err(|_| bad())?;
        let unit_len = after.chars().take_while(char::is_ascii_alphabetic).count();
        let (unit, after) = after.split_at(unit_len);
        let secs = match unit {
            "s" => 1,
            "m" => 60,
            "h" => 3600,
            "d" => 86_400,
            "w" => 7 * 86_400,
            "mo" => 30 * 86_400,
            "y" => 365 * 86_400,
            _ => return Err(bad()),
        };
        total += n * secs;
        rest = after.trim_start();
    }
    if t.is_empty() {
        return Err(bad());
    }
    Ok(total)
}

/// What a session's id says: its project and number.
pub fn session_ref(id: &str) -> Result<(&str, u32)> {
    let (project, number) = id
        .split_once('#')
        .with_context(|| format!("{id:?} isn't project#number"))?;
    Ok((
        project,
        number
            .parse()
            .with_context(|| format!("{id:?} isn't project#number"))?,
    ))
}

/// What a pull request's id says: its repository and number.
pub fn pull_ref(id: &str) -> Result<(&str, u64)> {
    let (repo, number) = id
        .split_once('#')
        .with_context(|| format!("{id:?} isn't owner/name#number"))?;
    if !repo.contains('/') {
        bail!("{id:?} isn't owner/name#number");
    }
    Ok((
        repo,
        number
            .parse()
            .with_context(|| format!("{id:?} isn't owner/name#number"))?,
    ))
}

impl Cast {
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let cast: Self =
            toml::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
        cast.check()
            .with_context(|| format!("checking {}", path.display()))?;
        Ok(cast)
    }

    pub fn person(&self, id: &str) -> Result<&Person> {
        self.people
            .iter()
            .find(|p| p.id == id)
            .with_context(|| format!("no person {id:?} in the cast"))
    }

    pub fn workspace(&self, slug: &str) -> Result<&Workspace> {
        self.workspaces
            .iter()
            .find(|w| w.slug == slug)
            .with_context(|| format!("no workspace {slug:?} in the cast"))
    }

    pub fn project(&self, slug: &str) -> Result<&Project> {
        self.projects
            .iter()
            .find(|p| p.slug == slug)
            .with_context(|| format!("no project {slug:?} in the cast"))
    }

    pub fn ticket(&self, key: &str) -> Result<&Ticket> {
        self.tickets
            .iter()
            .find(|t| t.key == key)
            .with_context(|| format!("no ticket {key:?} in the cast"))
    }

    pub fn schedule(&self, project: &str, name: &str) -> Result<&Schedule> {
        self.schedules
            .iter()
            .find(|s| s.project == project && s.name == name)
            .with_context(|| format!("no schedule {name:?} in {project}"))
    }

    pub fn session(&self, id: &str) -> Result<&SessionSpec> {
        self.sessions
            .iter()
            .find(|s| s.id == id)
            .with_context(|| format!("no session {id:?} in the cast"))
    }

    /// The session that opened pull request `id` (`owner/name#number`).
    pub fn pull_session(&self, id: &str) -> Option<&SessionSpec> {
        let (repo, number) = pull_ref(id).ok()?;
        self.sessions.iter().find(|s| {
            s.pr.as_ref().is_some_and(|p| p.number == number)
                && session_ref(&s.id)
                    .ok()
                    .and_then(|(p, _)| self.project(p).ok())
                    .and_then(|p| p.repo.as_deref())
                    == Some(repo)
        })
    }

    /// The people, workspaces, projects, runners and sessions it names all
    /// exist, and say what they can.
    fn check(&self) -> Result<()> {
        let mut ids = BTreeSet::new();
        for p in &self.people {
            if !ids.insert(p.id.as_str()) {
                bail!("two people are {:?}", p.id);
            }
            if p.avatar && p.github.is_none() {
                bail!(
                    "{}'s avatar is their GitHub one, but they have no `github`",
                    p.id
                );
            }
        }
        for w in &self.workspaces {
            for id in w.people() {
                self.person(id)
                    .with_context(|| format!("workspace {}", w.slug))?;
            }
            for id in w.joined.keys() {
                if !w.people().contains(&id.as_str()) {
                    bail!("{id} joined {}, but isn't in it", w.slug);
                }
            }
            if let Some(l) = &w.linear {
                self.person(&l.by)?;
            }
        }
        for r in &self.runners {
            self.person(&r.owner)
                .with_context(|| format!("runner {}", r.name))?;
            for w in &r.workspaces {
                let ws = self
                    .workspace(w)
                    .with_context(|| format!("runner {}", r.name))?;
                if !ws.people().contains(&r.owner.as_str()) {
                    bail!(
                        "runner {} runs {w}'s sessions, but its owner {} isn't in {w}",
                        r.name,
                        r.owner
                    );
                }
            }
            if !r.personal.iter().all(|w| r.workspaces.contains(w)) {
                bail!("runner {} is personal where it doesn't run", r.name);
            }
        }
        for p in &self.projects {
            let w = self
                .workspace(&p.workspace)
                .with_context(|| format!("project {}", p.slug))?;
            if p.by != w.owner && !w.admins.contains(&p.by) {
                bail!("{} creates {}, but only {}'s admins create its projects", p.by, p.slug, w.slug);
            }
            for id in std::iter::once(&p.by)
                .chain(&p.owners)
                .chain(&p.maintainers)
            {
                if !w.people().contains(&id.as_str()) {
                    bail!("{id} has a role in {}, but isn't in {}", p.slug, w.slug);
                }
            }
            // A project's repositories belong to one GitHub account, which
            // its workspace works in.
            if let Some(repo) = &p.repo {
                let owner = repo.split('/').next().unwrap_or_default();
                if !w.github.iter().any(|g| g == owner) {
                    bail!("{} is {owner}'s, which {} doesn't work in", p.slug, w.slug);
                }
                for g in &p.grants {
                    if g.repo.split('/').next() != Some(owner) {
                        bail!(
                            "{} grants {}, another account's than {repo}'s",
                            p.slug,
                            g.repo
                        );
                    }
                }
            }
            if !matches!(p.retros.len(), 0 | 3) {
                bail!("{}'s retros are [completed, partial, blocked]", p.slug);
            }
            // Reviews run on an owner's subscription.
            if let Some(r) = &p.reviews
                && !p.owners.contains(&r.operator)
                && !std::iter::once(&w.owner)
                    .chain(&w.admins)
                    .any(|a| *a == r.operator)
            {
                bail!(
                    "{}'s reviews run on {}'s subscription, who doesn't own it",
                    p.slug,
                    r.operator
                );
            }
        }
        let mut seen = BTreeSet::new();
        for s in &self.sessions {
            let (project, _) = session_ref(&s.id)?;
            let p = self
                .project(project)
                .with_context(|| format!("session {}", s.id))?;
            if !seen.insert(s.id.as_str()) {
                bail!("two sessions are {}", s.id);
            }
            let w = self.workspace(&p.workspace)?;
            if !w.people().contains(&s.by.as_str()) {
                bail!("{} started {}, but isn't in {}", s.by, s.id, w.slug);
            }
            if let Some(r) = &s.runner {
                let runner = self
                    .runners
                    .iter()
                    .find(|x| &x.name == r)
                    .with_context(|| format!("session {}: no runner {r}", s.id))?;
                if !runner.workspaces.contains(&p.workspace) {
                    bail!(
                        "{} runs on {r}, which doesn't run {}'s sessions",
                        s.id,
                        p.workspace
                    );
                }
                if runner.personal.contains(&p.workspace) && runner.owner != s.by {
                    bail!(
                        "{} runs on {r}, which runs only {}'s sessions there",
                        s.id,
                        runner.owner
                    );
                }
            }
            if s.title.is_none() && s.review.is_none() {
                bail!("{} has no title", s.id);
            }
            if (s.state == State::Stopped && s.took.is_none())
                || (s.state == State::Working && s.took.is_some())
            {
                bail!(
                    "{}: a stopped session says how long it `took`, and one still at work doesn't",
                    s.id
                );
            }
            if let Some(t) = &s.ticket {
                self.ticket(t)?;
            }
            if let Some(name) = &s.schedule {
                self.schedule(project, name)?;
            }
            if let Some(r) = &s.review {
                pull_ref(&r.of)?;
                if self.pull_session(&r.of).is_none() && !self.pulls.iter().any(|x| x.id == r.of) {
                    bail!(
                        "{} reviews {}, which neither a session nor a [[pull]] opened",
                        s.id,
                        r.of
                    );
                }
            }
            for m in &s.messages {
                if m.from != "fctl" {
                    self.person(&m.from)?;
                }
            }
        }
        // Numbers go up with time in each project.
        for p in &self.projects {
            let mut own: Vec<(u32, i64, &str)> = self
                .sessions
                .iter()
                .filter_map(|s| {
                    let (proj, n) = session_ref(&s.id).ok()?;
                    (proj == p.slug).then_some((n, s.start.offset(), s.id.as_str()))
                })
                .collect();
            own.sort();
            for pair in own.windows(2) {
                if let [(_, a, x), (_, b, y)] = pair
                    && b < a
                {
                    bail!("{y} starts before {x}, though its number is higher");
                }
            }
        }
        for n in &self.notes {
            self.project(&n.project)?;
            self.person(&n.by)?;
            if let Some(s) = n.replaced_by.as_ref().or(n.stale_by.as_ref()) {
                self.session(s)?;
            }
        }
        for s in &self.schedules {
            self.project(&s.project)?;
            self.person(&s.owner)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Dur, When, dur_of, offset_of};

    #[test]
    fn times_read_as_the_story_tells_them() {
        assert_eq!(offset_of("now"), Ok(0));
        assert_eq!(offset_of("-18m"), Ok(-18 * 60));
        assert_eq!(offset_of("today 08:52"), Ok(-18 * 60));
        assert_eq!(offset_of("yesterday 22:40"), Ok(-(10 * 3600 + 30 * 60)));
        assert_eq!(offset_of("Monday 22:40"), offset_of("yesterday 22:40"));
        assert_eq!(
            offset_of("saturday 03:00"),
            Ok(-(3 * 86_400 + 6 * 3600 + 10 * 60))
        );
        assert_eq!(offset_of("last thursday 09:10"), Ok(-5 * 86_400));
        assert_eq!(offset_of("last tuesday 09:10"), Ok(-7 * 86_400));
        assert_eq!(offset_of("-3w"), Ok(-21 * 86_400));
        assert!(offset_of("tomorrow 10:00").is_err());
        assert!(offset_of("today 25:00").is_err());
        assert!(offset_of("soon").is_err());
        assert_eq!(When("today 09:00".into()).offset(), -600);
    }

    #[test]
    fn lengths_of_time_add_up() {
        assert_eq!(dur_of("90s"), Ok(90));
        assert_eq!(dur_of("1m31s"), Ok(91));
        assert_eq!(dur_of("2h"), Ok(7200));
        assert_eq!(dur_of("14mo"), Ok(14 * 30 * 86_400));
        assert_eq!(dur_of("1h 5m"), Ok(3900));
        assert!(dur_of("").is_err());
        assert!(dur_of("5").is_err());
        assert!(dur_of("5x").is_err());
        assert_eq!(Dur::try_from("3y".to_owned()), Ok(Dur(3 * 365 * 86_400)));
    }
}
