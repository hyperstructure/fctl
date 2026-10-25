//! `cargo xtask demo`: the demo cast (`.claude/skills/demo`) in a stack of
//! its own, lived in, for screenshots and videos.
//!
//! It runs as `cargo xtask dev` does, with its state in `.fctl-demo/`
//! (made afresh each time), and loads `xtask/demo/cast.toml` into it in
//! three parts:
//!
//! 1. Through the API, as the cast's people: their accounts, workspaces,
//!    runners (a host for each one online, with a stand-in GitHub,
//!    `github.rs`), projects, schedules and notes, and every session,
//!    which fake-claude plays from its transcript (`xtask/demo/transcripts`)
//!    in the story's order (`world.rs`).
//! 2. With the control plane stopped, in its D1 and Durable Objects' SQLite:
//!    the story's times, and what a dev instance has no GitHub or Linear
//!    for: pull requests' checks, reviews, tickets (`rewrite.rs`).
//! 3. The control plane again, its hosts reconnected: the sessions still at
//!    work in the story work on, and the people watching them watch.

pub mod cast;
mod clock;
mod github;
mod rewrite;
mod sqlite;
mod world;

use crate::dev::{self, Children};
use crate::util;
use anyhow::{Context, Result, bail};
use cast::Cast;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

pub struct DemoOpts {
    /// Serve the built web app from wrangler instead of running Vite.
    pub no_vite: bool,
    /// Another cast than `xtask/demo/cast.toml`, with its transcripts,
    /// repositories and avatars beside it.
    pub cast: Option<std::path::PathBuf>,
}

/// How many times faster than written fake-claude plays the transcripts
/// while the seed runs them.
const SEEDING_SPEED: &str = "40";

/// Milliseconds since the epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// A moment as GitHub writes it: `2026-10-27T08:52:00Z`.
pub fn iso(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The date `days` after 1970-01-01 (Howard Hinnant's `civil_from_days`).
pub fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Runs SQL on the demo's local D1 while `wrangler dev` runs.
pub fn wrangler_sql(root: &Path, state: &Path, sql: &str) -> Result<()> {
    if sql.trim().is_empty() {
        return Ok(());
    }
    let file = state.join("seed.sql");
    std::fs::write(&file, sql)?;
    d1_execute(
        root,
        state,
        &[std::ffi::OsStr::new("--file"), file.as_os_str()],
    )?;
    Ok(())
}

/// Runs a query on the demo's local D1 while `wrangler dev` runs.
pub fn wrangler_query(root: &Path, state: &Path, sql: &str) -> Result<Vec<Value>> {
    let out = d1_execute(
        root,
        state,
        &[
            std::ffi::OsStr::new("--json"),
            "--command".as_ref(),
            sql.as_ref(),
        ],
    )?;
    let v: Value = serde_json::from_slice(&out).context("reading wrangler's answer")?;
    Ok(v.get(0)
        .and_then(|r| r.get("results"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// `wrangler d1 execute` on the demo's local D1, and what it printed. The
/// running stack writes to the same file: while it holds the lock, wrangler
/// finds the database busy, and tries again.
fn d1_execute(root: &Path, state: &Path, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let out = util::without_package_env(
            util::wrangler(root)?
                .args(["d1", "execute", "DB", "--local", "--persist-to"])
                .arg(state.join("wrangler"))
                .args(args)
                .env("CI", "1")
                .current_dir(root.join("crates/fctl-control")),
        )
        .output()
        .context("running wrangler d1 execute")?;
        if out.status.success() {
            return Ok(out.stdout);
        }
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if attempt < 8 && (said.contains("SQLITE_BUSY") || said.contains("database is locked")) {
            std::thread::sleep(Duration::from_millis(250 * attempt));
            continue;
        }
        bail!("wrangler d1 execute: {said}");
    }
}

/// Every session has its transcript, which answers its first message and
/// each later one.
fn check_transcripts(cast: &Cast, transcripts: &world::Transcripts) -> Result<()> {
    let mut wrong = Vec::new();
    for s in &cast.sessions {
        let name = world::transcript_of(s);
        let Some((first, turns)) = transcripts.matches.get(&name) else {
            wrong.push(format!("{}: no transcript {name}.toml", s.id));
            continue;
        };
        if let Some(p) = &s.prompt
            && !p.to_lowercase().contains(&first.to_lowercase())
        {
            wrong.push(format!(
                "{}'s prompt doesn't contain its transcript's match {first:?}",
                s.id
            ));
        }
        for m in &s.messages {
            let answered = turns.iter().skip(1).any(|t| match t {
                Some(t) => {
                    m.text.to_lowercase().contains(&t.to_lowercase())
                        || (m.review.is_some()
                            && t.to_lowercase().starts_with("fctl's review agent"))
                }
                None => true,
            });
            if !answered {
                wrong.push(format!(
                    "{}: no turn of {name}.toml answers {:?}",
                    s.id, m.text
                ));
            }
        }
    }
    if !wrong.is_empty() {
        bail!(
            "the transcripts don't play the cast:\n  {}",
            wrong.join("\n  ")
        );
    }
    Ok(())
}

/// Every edit in a transcript finds what it replaces, and every read
/// without a written `output` finds its file: in its project's repository
/// as the stand-in makes it (`repos/<owner>/<name>`), as the transcript's
/// earlier steps left it; a review's, as the session that opened the pull
/// request left it. Files a command names go unchecked after it, since
/// what it did to them isn't known.
fn check_changes(cast: &Cast, data: &Path) -> Result<()> {
    let mut wrong = Vec::new();
    let mut left: BTreeMap<&str, Files> = BTreeMap::new();
    // The sessions that open pull requests before the reviews of them.
    let (reviews, others): (Vec<_>, Vec<_>) =
        cast.sessions.iter().partition(|s| s.review.is_some());
    for s in others.into_iter().chain(reviews) {
        let name = world::transcript_of(s);
        let path = data.join("transcripts").join(format!("{name}.toml"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let v: toml::Value =
            toml::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
        let (project, _) = cast::session_ref(&s.id)?;
        let Some(repo) = &cast.project(project)?.repo else {
            continue;
        };
        let opened = s
            .review
            .as_ref()
            .and_then(|r| cast.pull_session(&r.of))
            .and_then(|o| left.get(o.id.as_str()));
        let mut files = Files {
            base: data.join("repos").join(repo),
            changed: opened.map(|o| o.changed.clone()).unwrap_or_default(),
            unknown: opened.map(|o| o.unknown.clone()).unwrap_or_default(),
        };
        let turns = v.get("turn").and_then(toml::Value::as_array);
        for turn in turns.into_iter().flatten() {
            let steps = turn.get("step").and_then(toml::Value::as_array);
            for step in steps.into_iter().flatten() {
                files.step(step, &name, &mut wrong);
            }
        }
        left.insert(s.id.as_str(), files);
    }
    if !wrong.is_empty() {
        bail!(
            "transcripts change what their repositories don't have:\n  {}",
            wrong.join("\n  ")
        );
    }
    Ok(())
}

/// A session's repository as its transcript changes it.
struct Files {
    base: std::path::PathBuf,
    changed: BTreeMap<String, String>,
    unknown: BTreeSet<String>,
}

impl Files {
    fn content(&self, path: &str) -> Option<String> {
        self.changed
            .get(path)
            .cloned()
            .or_else(|| std::fs::read_to_string(self.base.join(path)).ok())
    }

    fn step(&mut self, step: &toml::Value, name: &str, wrong: &mut Vec<String>) {
        let text = |k: &str| step.get(k).and_then(toml::Value::as_str);
        // Several calls at once, and a subagent's.
        for key in ["tools", "steps"] {
            for call in step
                .get(key)
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
            {
                self.step(call, name, wrong);
            }
        }
        for key in ["run", "silently"] {
            for word in text(key)
                .unwrap_or_default()
                .split(|c: char| c.is_whitespace() || "'\"<>|;&()".contains(c))
                .filter(|w| !w.is_empty())
            {
                self.unknown
                    .insert(word.trim_start_matches("./").to_owned());
            }
        }
        let relative = |path: &str| {
            (!path.starts_with('/') && !path.starts_with('~'))
                .then(|| path.trim_start_matches("./").to_owned())
        };
        if let Some(path) = text("write").and_then(relative) {
            self.changed
                .insert(path, text("content").unwrap_or_default().to_owned());
        }
        if let Some(path) = text("edit").and_then(relative)
            && !self.unknown.contains(&path)
        {
            let (old, new) = (
                text("old").unwrap_or_default(),
                text("new").unwrap_or_default(),
            );
            match self.content(&path) {
                Some(c) if c.contains(old) => {
                    let all = step
                        .get("replace_all")
                        .and_then(toml::Value::as_bool)
                        .unwrap_or(false);
                    let c = if all {
                        c.replace(old, new)
                    } else {
                        c.replacen(old, new, 1)
                    };
                    self.changed.insert(path, c);
                }
                Some(_) => wrong.push(format!("{name}: the edit of {path} doesn't find {old:?}")),
                None => wrong.push(format!("{name}: an edit of {path}, which isn't there")),
            }
        }
        if let Some(path) = text("read").and_then(relative)
            && step.get("output").is_none()
            && !self.unknown.contains(&path)
            && self.content(&path).is_none()
        {
            wrong.push(format!(
                "{name}: a read of {path}, which isn't there: add the file, or give the read an `output`"
            ));
        }
    }
}

/// The demo's control plane: its state in `.fctl-demo/wrangler`, as fctl
/// rather than "fctl dev", with a Linear app made up.
fn wrangler(root: &Path, state: &Path, cast: &Cast, children: &mut Children) -> Result<()> {
    let vars = [
        ("FCTL_INSTANCE_NAME", cast.instance.name.clone()),
        ("FCTL_EARLY_ACCESS", "0".to_owned()),
        ("LINEAR_CLIENT_ID", "demo-linear-client".to_owned()),
        ("LINEAR_CLIENT_SECRET", "demo-linear-secret".to_owned()),
        (
            "LINEAR_WEBHOOK_SECRET",
            "demo-linear-webhook-secret".to_owned(),
        ),
    ];
    dev::wrangler_dev(root, children, Some(&state.join("wrangler")), &vars)?;
    dev::wait_for(
        &format!("{}/api/v1/health", dev::LOCAL_API),
        Duration::from_secs(300),
        children,
    )
}

/// What `xtask/demo/shoot.mjs` needs: where to sign in as whom, the
/// sessions and workspaces by the cast's names, and the Linear teams the
/// dev stack can't ask Linear for.
fn shots(
    cast: &Cast,
    world: &world::World,
    story: &clock::Story,
    state: &Path,
    app: &str,
) -> Result<()> {
    let noor = cast.person("noor")?;
    let sessions: serde_json::Map<String, Value> = world
        .sessions
        .iter()
        .map(|(k, m)| (k.clone(), Value::String(m.id.clone())))
        .collect();
    let mut teams = serde_json::Map::new();
    for w in &cast.workspaces {
        let Some(l) = &w.linear else { continue };
        let list: Vec<Value> = l
            .teams
            .iter()
            .map(|t| {
                let state = |name: &str, kind: &str| {
                    serde_json::json!({ "name": name, "kind": kind, "id": format!("lin-state-{}-{}", t.key.to_lowercase(), kind) })
                };
                serde_json::json!({
                    "id": format!("lin-team-{}", t.key.to_lowercase()),
                    "key": t.key,
                    "name": t.name,
                    "color": t.color,
                    "states": [state("Backlog", "backlog"), state("Todo", "unstarted"), state("In Progress", "started"),
                               state("In Review", "started"), state("Done", "completed"), state("Canceled", "canceled")],
                    "start_state": format!("lin-state-{}-started", t.key.to_lowercase()),
                    "projects": [{ "id": "lin-project-copenhagen", "name": "Copenhagen launch" }],
                })
            })
            .collect();
        teams.insert(world.workspaces[&w.slug].clone(), Value::Array(list));
    }
    let manifest = serde_json::json!({
        "app": app,
        "email": noor.email,
        "password": cast.instance.password,
        "sessions": sessions,
        "workspaces": world.workspaces,
        "teams": teams,
        "anchor": story.anchor,
        "zone": story.zone,
    });
    std::fs::write(
        state.join("shots.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    Ok(())
}

/// Stops what an earlier run left running in `state`: its sessions'
/// agents and fake-claudes, whose command lines name it.
fn stop_leftovers(state: &Path) {
    let Ok(out) = Command::new("pgrep")
        .arg("-f")
        .arg(state.display().to_string())
        .output()
    else {
        return;
    };
    let me = std::process::id().to_string();
    for pid in String::from_utf8_lossy(&out.stdout).split_whitespace() {
        if pid != me {
            let _ = Command::new("kill").arg(pid).status();
        }
    }
}

pub fn demo(root: &Path, opts: DemoOpts) -> Result<()> {
    let state = root.join(".fctl-demo");
    let seeded = seed(root, opts, &state);
    if seeded.is_err() {
        // Its sessions' agents outlive it, in process groups of their own.
        stop_leftovers(&state);
    }
    seeded
}

fn seed(root: &Path, opts: DemoOpts, state: &Path) -> Result<()> {
    let file = opts
        .cast
        .clone()
        .unwrap_or_else(|| root.join("xtask/demo/cast.toml"));
    let data = file
        .parent()
        .map_or_else(|| root.join("xtask/demo"), Path::to_path_buf);
    let cast = Cast::load(&file)?;
    let transcripts = world::Transcripts::load(&data.join("transcripts"))?;
    check_transcripts(&cast, &transcripts)?;
    check_changes(&cast, &data)?;
    // The agents of an earlier run's sessions still at work outlive their
    // hosts (they have process groups of their own).
    stop_leftovers(state);
    if state.exists() {
        std::fs::remove_dir_all(state).with_context(|| format!("removing {}", state.display()))?;
    }
    std::fs::create_dir_all(state)?;
    let started = now_ms();
    dev::build(root, true, opts.no_vite, Some(&state.join("wrangler")))?;
    // The stand-in GitHub, with the cast's repositories.
    let github = github::Github::new(state);
    github.install()?;
    for r in &cast.repos {
        let by = cast.person(&r.by)?;
        github.create(
            &r.name,
            Some(&data.join("repos").join(&r.name)),
            (&by.name(), &by.email),
            (started / 1000) + r.created.offset(),
        )?;
    }
    // fake-claude's replays, played fast while the seed runs them.
    let replays = state.join("replays");
    std::fs::create_dir_all(&replays)?;
    for name in transcripts.matches.keys() {
        std::fs::copy(
            transcripts.dir.join(format!("{name}.toml")),
            replays.join(format!("{name}.toml")),
        )?;
    }
    std::fs::write(replays.join("speed"), SEEDING_SPEED)?;
    let env: String = github
        .git_env()
        .iter()
        .map(|(k, v)| format!("{k}={v}\n"))
        .collect();
    std::fs::write(replays.join("env"), env)?;

    let mut control = Children(Vec::new());
    wrangler(root, state, &cast, &mut control)?;
    let mut others = Children(Vec::new());
    if !opts.no_vite {
        let vite = util::without_package_env(
            Command::new(root.join("node_modules/.bin/vite"))
                .args(["--host", "localhost"])
                .current_dir(root.join("web")),
        )
        .spawn()
        .context("starting vite")?;
        others.0.push(("vite", vite));
    }

    let mut world = world::World::new(&cast, root, state, &github, transcripts);
    let step = |what: &str| eprintln!("\x1b[1mdemo\x1b[0m {what}");
    step("people");
    world.accounts()?;
    step("workspaces");
    world.workspaces(&data)?;
    step("runners and their hosts");
    // Runners offline in the story run their sessions too, and stop after.
    let mut offline = Children(Vec::new());
    world.runners(&mut others, &mut offline)?;
    step("projects");
    world.projects()?;
    world.connections()?;
    step("schedules and notes");
    world.schedules()?;
    world.notes()?;
    step("sessions, as the story has them");
    world.sessions(&mut others)?;
    world.drains()?;
    drop(offline);

    step("the story's times");
    drop(control);
    // The moment of the shots: just now, at 09:10 somewhere.
    let story = clock::Story::at(now_ms());
    rewrite::rewrite(&world, &story)?;
    let mut control = Children(Vec::new());
    wrangler(root, state, &cast, &mut control)?;
    world.until_hosts_online(&mut others, false)?;
    rewrite::machines(&world)?;
    std::fs::write(replays.join("speed"), "1")?;
    // The people watching the sessions they watch in the shots.
    for s in &cast.sessions {
        for who in &s.watching {
            let made = &world.sessions[&s.id];
            let viewer = Command::new(util::target_dir(root).join("debug/fctl"))
                .args(["attach", "--read-only", &made.id])
                .env("FCTL_SERVER", dev::LOCAL_API)
                .env("FCTL_TOKEN", &world.tokens[who])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .context("starting a viewer")?;
            others.0.push(("viewer", viewer));
        }
    }
    let noor = cast.person("noor")?;
    let open = if opts.no_vite {
        dev::LOCAL_API
    } else {
        dev::WEB
    };
    shots(&cast, &world, &story, state, open)?;
    let ago = (now_ms() - story.anchor) / 60_000;
    eprintln!(
        "\n\x1b[1mthe demo is running\x1b[0m\n\n  \
         app       {open}\n  \
         sign in   {} / {} (everyone's password)\n  \
         story     09:10 in {}, {ago} minutes ago: the times are the story's in that zone at that moment\n  \
         shoot     node xtask/demo/shoot.mjs, which shows them so (see .claude/skills/demo/SKILL.md)\n\n\
         Ctrl-C stops it. State lives in .fctl-demo/; the next run stops what is left of it and starts afresh.\n",
        noor.email, cast.instance.password, story.zone,
    );
    loop {
        control.check()?;
        others.check()?;
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::{Cast, check_changes, check_transcripts, civil, iso, world};
    use std::path::Path;

    /// The cast says what it can, every session has its transcript, and
    /// the transcripts change only what their repositories have.
    #[test]
    fn the_cast_holds_together() -> anyhow::Result<()> {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("demo");
        let cast = Cast::load(&data.join("cast.toml"))?;
        let transcripts = world::Transcripts::load(&data.join("transcripts"))?;
        check_changes(&cast, &data)?;
        check_transcripts(&cast, &transcripts)?;
        for p in cast.people.iter().filter(|p| p.avatar) {
            let login = p.github.as_deref().unwrap_or_default();
            assert!(
                data.join("avatars").join(format!("{login}.png")).exists(),
                "no avatar for {login}"
            );
        }
        for w in &cast.workspaces {
            if let Some(file) = &w.avatar {
                assert!(data.join("avatars").join(file).exists(), "no avatar {file}");
            }
        }
        Ok(())
    }

    #[test]
    fn dates_are_civil() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        // 2026-10-27T09:10:00Z
        assert_eq!(iso(1_793_092_200_000), "2026-10-27T09:10:00Z");
        assert_eq!(civil(-1), (1969, 12, 31));
    }
}
