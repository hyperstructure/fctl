//! The second part of the seed, with the control plane stopped: the
//! story's times in D1 and in the sessions' own SQLite, and what a dev
//! instance has no GitHub or Linear for.
//!
//! Each session's real times (seconds, as fake-claude played it fast) map
//! onto the story's: its start, the messages it got and the end of its last
//! turn are fixed points, and the times between them stretch to fit
//! ([`Clock`]). Every time in what it stored moves with them.

use super::cast::{self, SessionSpec, State};
use super::clock::Story;
use super::sqlite::{self, Db};
use super::world::World;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// How a session's real times map onto the story's: fixed points, with
/// the times between them stretched, and those beyond the last moved by as
/// much as it moved.
#[derive(Clone, Debug)]
pub struct Clock {
    points: Vec<(i64, i64)>,
}

impl Clock {
    /// From fixed points (real, story), which keep only those after the
    /// one before them in both.
    pub fn new(mut points: Vec<(i64, i64)>) -> Self {
        points.sort_by_key(|(r, _)| *r);
        let mut kept: Vec<(i64, i64)> = Vec::new();
        for (r, t) in points {
            match kept.last() {
                Some(&(lr, lt)) if r <= lr || t <= lt => {}
                _ => kept.push((r, t)),
            }
        }
        Self { points: kept }
    }

    pub fn map(&self, t: i64) -> i64 {
        let (Some(&(r0, t0)), Some(&(rn, tn))) = (self.points.first(), self.points.last()) else {
            return t;
        };
        if t <= r0 {
            return t0 - (r0 - t);
        }
        if t >= rn {
            return tn + (t - rn);
        }
        for pair in self.points.windows(2) {
            if let [(ra, ta), (rb, tb)] = pair
                && t >= *ra
                && t <= *rb
            {
                let f = (t - ra) as f64 / (rb - ra).max(1) as f64;
                return ta + ((tb - ta) as f64 * f) as i64;
            }
        }
        t
    }
}

/// Moves every time in `v` that falls in `window` (milliseconds, and
/// seconds as Claude's rate limits give them) as `clock` does.
pub fn remap(v: &mut Value, clock: &Clock, window: (i64, i64)) {
    match v {
        Value::Number(n) => {
            if let Some(x) = n.as_i64() {
                if (window.0..=window.1).contains(&x) {
                    *v = json!(clock.map(x));
                } else if (window.0 / 1000..=window.1 / 1000).contains(&x) {
                    *v = json!(clock.map(x * 1000) / 1000);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| remap(x, clock, window)),
        Value::Object(m) => m.values_mut().for_each(|x| remap(x, clock, window)),
        _ => {}
    }
}

/// A session's SQLite, by session id.
fn session_objects(state: &std::path::Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut out = BTreeMap::new();
    for path in sqlite::objects(state, "SessionObject")? {
        let rows = Db::open(&path).query("SELECT value FROM meta WHERE key = 'summary'", &[])?;
        let Some(text) = rows
            .first()
            .and_then(|r| r.get("value"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let v: Value = serde_json::from_str(text)?;
        if let Some(id) = v["id"].as_str() {
            out.insert(id.to_owned(), path);
        }
    }
    Ok(out)
}

struct Ctx<'w, 'a> {
    w: &'w World<'a>,
    /// The moment of the shots, and the zone whose clocks read 09:10 then.
    story: &'w Story,
    anchor: i64,
    /// Where real times fell: from just before the seed started to just
    /// after the control plane is back.
    window: (i64, i64),
}

impl Ctx<'_, '_> {
    /// When the story has `when`.
    fn at(&self, when: &cast::When) -> i64 {
        self.anchor + when.offset() * 1000
    }

    fn user(&self, person: &str) -> Value {
        let p = self.w.cast.person(person).ok();
        json!({
            "id": self.w.users.get(person).cloned().unwrap_or_default(),
            "name": p.map(cast::Person::name).unwrap_or_default(),
            "avatar_url": p.filter(|p| p.avatar).and_then(|p| p.github.as_ref()).map(|g| format!("https://avatars.githubusercontent.com/{g}")),
        })
    }
}

pub fn rewrite(w: &World, story: &Story) -> Result<()> {
    let state = w.state.join("wrangler");
    let anchor = story.anchor;
    let ctx = Ctx {
        w,
        story,
        anchor,
        window: (w.started - 3_600_000, anchor + 6 * 3_600_000),
    };
    let mut d1 = Db::open(&sqlite::d1(&state)?);
    let objects = session_objects(&state)?;
    let mut clocks: BTreeMap<String, Clock> = BTreeMap::new();
    for s in &w.cast.sessions {
        let made = &w.sessions[&s.id];
        let path = objects
            .get(&made.id)
            .with_context(|| format!("no stored session for {}", s.id))?;
        let clock =
            session(&ctx, s, path, &mut d1).with_context(|| format!("rewriting {}", s.id))?;
        clocks.insert(made.id.clone(), clock);
    }
    reviews(&ctx, &mut d1)?;
    fillers(&ctx, &mut d1)?;
    memory(&ctx, &clocks, &mut d1)?;
    schedules(&ctx, &mut d1)?;
    people_and_places(&ctx, &mut d1)?;
    runners(&ctx, &state, &mut d1)?;
    audit(&ctx, &clocks, &mut d1)?;
    d1.commit()?;
    Ok(())
}

/// One session: its events and summary in its own SQLite, and its row in D1.
fn session(ctx: &Ctx, s: &SessionSpec, path: &std::path::Path, d1: &mut Db) -> Result<Clock> {
    let w = ctx.w;
    let made = &w.sessions[&s.id];
    let (project, _) = cast::session_ref(&s.id)?;
    let p = w.cast.project(project)?;
    let mut obj = Db::open(path);
    let summary_text = obj
        .query("SELECT value FROM meta WHERE key = 'summary'", &[])?
        .first()
        .and_then(|r| r.get("value"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .context("no summary")?;
    let mut summary: Value = serde_json::from_str(&summary_text)?;
    let events = obj.query("SELECT seq, ts, kind, data FROM events ORDER BY seq", &[])?;
    let ts = |r: &sqlite::Row| r.get("ts").and_then(Value::as_i64).unwrap_or_default();
    let kind = |r: &sqlite::Row| {
        r.get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };

    // The fixed points: its start, each later message, the end of its
    // last turn (or, still at work, of what it stored before the shots).
    let created = summary["created_at"].as_i64().unwrap_or_default();
    let mut points = vec![(created, ctx.at(&s.start))];
    let messages: Vec<i64> = events
        .iter()
        .filter(|r| kind(r) == "user_message")
        .map(ts)
        .skip(1)
        .collect();
    for (m, spec) in messages.iter().zip(&s.messages) {
        points.push((*m, ctx.at(&spec.at)));
    }
    let last_result = events
        .iter()
        .rev()
        .filter(|r| {
            kind(r) == "claude"
                && r.get("data")
                    .and_then(Value::as_str)
                    .is_some_and(|d| d.contains("\"type\":\"result\""))
        })
        .map(ts)
        .next();
    let last = events.iter().map(ts).max().unwrap_or(created);
    let end_story = match (s.took, s.state) {
        (Some(took), _) => ctx.at(&s.start) + took.0 * 1000,
        // Still at work: what it stored last is from a few minutes before
        // the shots.
        _ => ctx.anchor - 3 * 60_000,
    };
    let end_real = match s.state {
        State::Stopped => last_result.unwrap_or(last),
        _ => last,
    };
    points.push((end_real, end_story));
    let clock = Clock::new(points);

    // Its events.
    let mut first_message = true;
    let mut message_index = 0usize;
    for r in &events {
        let seq = r.get("seq").and_then(Value::as_i64).unwrap_or_default();
        let k = kind(r);
        let mut data: Value = r
            .get("data")
            .and_then(Value::as_str)
            .and_then(|d| serde_json::from_str(d).ok())
            .unwrap_or(Value::Null);
        remap(&mut data, &clock, ctx.window);
        if k == "user_message" {
            if first_message {
                first_user_message(ctx, s, &mut data)?;
                first_message = false;
            } else {
                if let Some(m) = s.messages.get(message_index) {
                    later_message(ctx, s, m, &mut data)?;
                }
                message_index += 1;
            }
            // What the apps read: it has to be a message still.
            serde_json::from_value::<fctl_proto::event::UserMessage>(data.clone())
                .with_context(|| format!("a message of {} no longer reads as one", s.id))?;
        }
        obj.exec(
            "UPDATE events SET ts = ?, data = ? WHERE seq = ?",
            vec![json!(clock.map(ts(r))), json!(data.to_string()), json!(seq)],
        );
    }

    // Its summary.
    remap(&mut summary, &clock, ctx.window);
    if let Some(r) = &s.runner {
        summary["runner"] = json!({ "id": w.runners[r], "name": r });
    }
    // Paused at the plan's limit: until it resets, as the plan says.
    if s.state == State::Paused
        && let Some(left) = w
            .cast
            .person(&s.by)?
            .claude
            .iter()
            .find_map(|c| c.usage.and_then(|u| u.limited))
    {
        summary["resume_at"] = json!(ctx.anchor + left.0 * 1000);
    }
    if let (Some(pr), Some(repo)) = (&s.pr, &p.repo) {
        let refs = pull_requests(ctx, s, pr, repo, &summary)?;
        summary["pull_request"] = refs[0].clone();
        summary["pull_requests"] = json!(refs);
    }
    if let Some(key) = &s.ticket {
        summary["ticket"] = session_ticket(ctx, s, key)?;
    }
    if let Some(r) = &s.review {
        let (repo, number) = cast::pull_ref(&r.of)?;
        let (owner, name) = repo.split_once('/').context("owner/name")?;
        let sha = w.reviewed_sha(s)?;
        let title = pull_title(ctx, &r.of)?;
        summary["review"] = json!({
            "repo": { "owner": owner, "name": name }, "number": number,
            "url": format!("https://github.com/{repo}/pull/{number}"), "title": title, "head_sha": sha,
        });
        summary["title"] = json!(review_title(name, number, &title));
        summary["github_access"] = json!("read");
        let branch = review_branch(ctx, &r.of)?;
        summary["branch"] = json!(branch);
        let state = w
            .cast
            .pulls
            .iter()
            .find(|p| p.id == r.of)
            .map_or("open", |p| p.state.as_str());
        summary["pull_requests"] = json!([{
            "number": number, "url": format!("https://github.com/{repo}/pull/{number}"), "title": title,
            "state": state, "repo": { "owner": owner, "name": name },
        }]);
        summary["pull_request"] = summary["pull_requests"][0].clone();
    }
    let session: fctl_proto::session::Session = serde_json::from_value(summary.clone())
        .with_context(|| format!("{}'s summary no longer reads as a session", s.id))?;
    obj.exec(
        "UPDATE meta SET value = ? WHERE key = 'summary'",
        vec![json!(summary.to_string())],
    );
    obj.commit()?;

    // Its row, as the session object writes it through, with what only
    // its creation sets.
    let pr = session.pull_request.as_ref();
    let prs = (!session.pull_requests.is_empty())
        .then(|| serde_json::to_string(&session.pull_requests).ok())
        .flatten();
    d1.exec(
        "UPDATE sessions SET title = ?, description = ?, status = ?, status_reason = ?, resume_at = ?, model = COALESCE(?, model), \
         claude_effort = ?, branch = COALESCE(?, branch), remote_control = ?, stats = ?, pr_number = ?, pr_url = ?, pr_title = ?, pr_state = ?, \
         pull_requests = ?, updated_at = ?, last_activity_at = ?, started_at = ?, ended_at = ?, created_at = ?, runner_id = COALESCE(?, runner_id), \
         ticket = COALESCE(?, ticket), github_access = COALESCE(?, github_access) WHERE id = ?",
        vec![
            json!(session.title),
            json!(session.description),
            json!(session.status.as_str()),
            json!(session.status_reason),
            json!(session.resume_at),
            json!(session.model),
            json!(session.effort),
            json!(session.branch),
            json!(session.remote_control.as_ref().and_then(|r| serde_json::to_string(r).ok())),
            json!(serde_json::to_string(&session.stats)?),
            json!(pr.map(|p| p.number)),
            json!(pr.map(|p| p.url.clone())),
            json!(pr.and_then(|p| p.title.clone())),
            json!(pr.and_then(|p| p.state.clone())),
            json!(prs),
            json!(session.updated_at),
            json!(session.last_activity_at),
            json!(session.started_at),
            json!(session.ended_at),
            json!(session.created_at),
            json!(session.runner.as_ref().map(|r| r.id.0.clone())),
            json!(session.ticket.as_ref().and_then(|t| serde_json::to_string(t).ok())),
            json!(s.review.as_ref().map(|_| "read")),
            json!(made.id),
        ],
    );
    // Its published screenshots, memory and retrospectives move with it,
    // in `memory`.
    Ok(clock)
}

/// A review session's title, as fctl gives it.
fn review_title(repo_name: &str, number: u64, title: &str) -> String {
    let t = format!("Review of {repo_name}#{number}: {title}");
    if t.chars().count() <= 80 {
        t
    } else {
        let cut: String = t.chars().take(79).collect();
        format!("{cut}…")
    }
}

/// The branch of pull request `of`: its session's, or one a bot or an
/// outside contributor would push.
fn review_branch(ctx: &Ctx, of: &str) -> Result<String> {
    if let Some(s) = ctx.w.cast.pull_session(of) {
        return ctx.w.sessions[&s.id].branch.clone().context("no branch");
    }
    let p = ctx
        .w
        .cast
        .pulls
        .iter()
        .find(|p| p.id == of)
        .context("no pull request")?;
    let slug = super::world::slug(&p.title);
    Ok(if p.author.contains("dependabot") {
        format!("dependabot/{slug}")
    } else if p.author.contains("renovate") {
        format!("renovate/{slug}")
    } else {
        format!("{}/{slug}", p.author)
    })
}

fn pull_title(ctx: &Ctx, of: &str) -> Result<String> {
    if let Some(s) = ctx.w.cast.pull_session(of) {
        return Ok(s
            .pr
            .as_ref()
            .and_then(|p| p.title.clone())
            .or_else(|| s.title.clone())
            .unwrap_or_default());
    }
    ctx.w
        .cast
        .pulls
        .iter()
        .find(|p| p.id == of)
        .map(|p| p.title.clone())
        .context("no pull request")
}

/// The pull requests a session lists: the one it opened, as GitHub has it
/// at the shots, with its checks.
fn pull_requests(
    ctx: &Ctx,
    s: &SessionSpec,
    pr: &cast::SessionPull,
    repo: &str,
    summary: &Value,
) -> Result<Vec<Value>> {
    let (owner, name) = repo.split_once('/').context("owner/name")?;
    let url = format!("https://github.com/{repo}/pull/{}", pr.number);
    let title = pr
        .title
        .clone()
        .or_else(|| s.title.clone())
        .unwrap_or_default();
    let opened = summary["pull_requests"]
        .as_array()
        .and_then(|l| l.iter().find(|x| x["number"].as_u64() == Some(pr.number)))
        .and_then(|x| x["state_changed_at"].as_i64().or(x["updated_at"].as_i64()))
        .unwrap_or_else(|| ctx.at(&s.start) + 60_000);
    let changed = pr.at.as_ref().map_or(opened, |a| ctx.at(a));
    let mut out = json!({
        "number": pr.number, "url": url, "title": title, "state": pr.state,
        "repo": { "owner": owner, "name": name },
        "state_changed_at": changed, "updated_at": changed,
    });
    if pr.passed.is_some() || pr.failed.is_some() || pr.pending.is_some() {
        let branch = ctx.w.sessions[&s.id].branch.clone().unwrap_or_default();
        let sha = ctx.w.github.commits(repo, &branch)?.last().cloned();
        out["checks"] = json!({
            "passed": pr.passed.unwrap_or(0), "failed": pr.failed.unwrap_or(0), "pending": pr.pending.unwrap_or(0),
            "sha": sha, "at": changed.max(ctx.anchor - 120_000),
        });
    }
    Ok(vec![out])
}

/// The ticket a session started from, as Linear names it.
fn session_ticket(ctx: &Ctx, s: &SessionSpec, key: &str) -> Result<Value> {
    let t = ctx.w.cast.ticket(key)?;
    let (project, _) = cast::session_ref(&s.id)?;
    let w = ctx
        .w
        .cast
        .workspace(&ctx.w.cast.project(project)?.workspace)?;
    let l = w.linear.as_ref().context("no Linear")?;
    Ok(json!({
        "tracker": "linear", "id": format!("lin-issue-{}", key.to_lowercase()), "key": key, "title": t.title,
        "url": format!("https://linear.app/{}/issue/{key}/{}", l.url_key, super::world::slug(&t.title)),
    }))
}

/// The session's first message, as the way it started has it: a ticket
/// delegated in Linear, a review fctl started, or a person in the app.
fn first_user_message(ctx: &Ctx, s: &SessionSpec, data: &mut Value) -> Result<()> {
    if let Some(key) = &s.ticket {
        let t = ctx.w.cast.ticket(key)?;
        let by = ctx.w.cast.person(&t.by)?;
        data["source"] = json!("api");
        data["ticket"] = json!({
            "ticket": session_ticket(ctx, s, key)?,
            "event": "delegated",
            "by": { "name": by.name(), "handle": by.id, "avatar_url": by.github.as_ref().filter(|_| by.avatar).map(|g| format!("https://avatars.githubusercontent.com/{g}")), "app": false },
            "description": t.description,
            // The ticket's state when it came.
            "state": { "name": "Todo", "kind": "unstarted" },
            "labels": t.labels,
            "explore": t.labels.iter().any(|l| l == "explore"),
        });
    } else if s.review.is_some() {
        data["author"] = Value::Null;
        data["source"] = json!("system");
    } else if s.schedule.is_some() {
        // fctl started it at the schedule's time: as it was.
    } else {
        data["source"] = json!("web");
    }
    Ok(())
}

/// A later message: fctl's passing on of a review's findings, or a person's.
fn later_message(ctx: &Ctx, s: &SessionSpec, m: &cast::Message, data: &mut Value) -> Result<()> {
    if m.from != "fctl" {
        data["author"] = ctx.user(&m.from);
        data["source"] = json!("web");
        return Ok(());
    }
    data["author"] = Value::Null;
    data["source"] = json!("system");
    let Some(review) = &m.review else {
        return Ok(());
    };
    let r = ctx.w.cast.session(review)?;
    let spec = r.review.as_ref().context("not a review")?;
    let (repo, number) = cast::pull_ref(&spec.of)?;
    let (owner, name) = repo.split_once('/').context("owner/name")?;
    data["review"] = json!({
        "repo": { "owner": owner, "name": name }, "number": number,
        "url": format!("https://github.com/{repo}/pull/{number}"),
        "title": pull_title(ctx, &spec.of)?,
        "sha": ctx.w.reviewed_sha(r)?,
        "summary_url": format!("https://github.com/{repo}/pull/{number}#pullrequestreview-{}", 2_000_000 + number),
        "session": ctx.w.sessions[&r.id].id,
        "findings": m.findings.iter().map(|f| json!({
            "severity": f.severity, "title": f.title, "path": f.path, "line": f.line, "deleted": false,
        })).collect::<Vec<_>>(),
    });
    let _ = s;
    Ok(())
}

/// The reviews: a row for each review session, as review agents write them.
fn reviews(ctx: &Ctx, d1: &mut Db) -> Result<()> {
    let w = ctx.w;
    // Each pull request's reviews in order, for what came before each.
    let mut before: BTreeMap<String, String> = BTreeMap::new();
    let mut ordered: Vec<&SessionSpec> = w
        .cast
        .sessions
        .iter()
        .filter(|s| s.review.is_some())
        .collect();
    ordered.sort_by_key(|s| s.start.offset());
    for s in ordered {
        let r = s.review.as_ref().context("a review")?;
        let (project, _) = cast::session_ref(&s.id)?;
        let pid = &w.projects[project];
        let (repo, number) = cast::pull_ref(&r.of)?;
        let sha = w.reviewed_sha(s)?;
        let author = match w.cast.pull_session(&r.of) {
            Some(_) => format!("{}[bot]", w.cast.instance.github_app),
            None => w
                .cast
                .pulls
                .iter()
                .find(|p| p.id == r.of)
                .map(|p| p.author.clone())
                .unwrap_or_default(),
        };
        let outside = !author.ends_with("[bot]");
        let start = ctx.at(&s.start);
        let finished = r.took.map(|t| start + t.0 * 1000);
        let reason = (r.result == "superseded").then(|| {
            let next = w
                .cast
                .sessions
                .iter()
                .filter(|x| x.review.as_ref().is_some_and(|y| y.of == r.of))
                .find(|x| x.start.offset() > s.start.offset())
                .and_then(|x| w.reviewed_sha(x).ok())
                .unwrap_or_default();
            format!(
                "A newer commit came, {}: fctl reviews that one instead.",
                &next[..7.min(next.len())]
            )
        });
        let posted = (r.result == "passed" || r.result == "commented").then(|| {
            format!(
                "https://github.com/{repo}/pull/{number}#pullrequestreview-{}",
                2_000_000 + number + u64::from(r.findings) * 7 + start.unsigned_abs() % 97
            )
        });
        d1.exec(
            "INSERT INTO reviews (session_id, project_id, repo, repo_key, number, head_sha, attempt, url, title, author, draft, fork, trusted, \
             previous_sha, pr_updated_at, installation_id, state, reason, findings, resolved, still_open, review_url, created_at, finished_at) \
             VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?, ?, 0, ?, ?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                json!(w.sessions[&s.id].id),
                json!(pid),
                json!(repo),
                json!(repo.to_ascii_lowercase()),
                json!(number),
                json!(sha),
                json!(format!("https://github.com/{repo}/pull/{number}")),
                json!(pull_title(ctx, &r.of)?),
                json!(author),
                json!(i64::from(outside)),
                json!(i64::from(!outside)),
                json!(before.get(&r.of)),
                json!(w.cast.pulls.iter().find(|p| p.id == r.of).map_or(start - 60_000, |p| ctx.at(&p.at))),
                json!(r.result),
                json!(reason),
                json!(r.findings),
                json!(r.resolved),
                json!(r.still_open),
                json!(posted),
                json!(start),
                json!(finished),
            ],
        );
        before.insert(r.of.clone(), sha);
    }
    Ok(())
}

/// Notes and retrospectives agents wrote move with their sessions; the
/// people's notes take the story's times; notes others replaced go stale.
fn memory(ctx: &Ctx, clocks: &BTreeMap<String, Clock>, d1: &mut Db) -> Result<()> {
    let w = ctx.w;
    let rows = d1.query(
        "SELECT id, session_id, created_at, text, project_id FROM memories",
        &[],
    )?;
    let rows: Vec<(String, Option<String>, i64, String, String)> = rows
        .iter()
        .map(|r| {
            (
                r.get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                r.get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                r.get("created_at")
                    .and_then(Value::as_i64)
                    .unwrap_or_default(),
                r.get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                r.get("project_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            )
        })
        .collect();
    let mut created: BTreeMap<String, i64> = BTreeMap::new();
    for (id, session, at, text, _) in &rows {
        let new = match session {
            Some(sid) => clocks.get(sid).map_or(*at, |c| c.map(*at)),
            None => w
                .cast
                .notes
                .iter()
                .find(|n| n.text == *text)
                .map_or(*at, |n| ctx.at(&n.at)),
        };
        created.insert(id.clone(), new);
        d1.exec(
            "UPDATE memories SET created_at = ? WHERE id = ?",
            vec![json!(new), json!(id)],
        );
    }
    for n in &w.cast.notes {
        let Some((id, _, _, _, project)) = rows.iter().find(|r| r.1.is_none() && r.3 == n.text)
        else {
            continue;
        };
        if let Some(by) = &n.replaced_by {
            let sid = &w.sessions[by].id;
            // The note of that session's that says most of what it said.
            let replacement = rows
                .iter()
                .filter(|r| r.1.as_deref() == Some(sid.as_str()) && &r.4 == project)
                .max_by_key(|r| common_prefix(&r.3, &n.text))
                .map(|r| r.0.clone());
            let starter = w.cast.session(by)?.by.clone();
            d1.exec(
                "UPDATE memories SET stale_at = ?, stale_session_id = ?, stale_by = ?, replaced_by = ? WHERE id = ?",
                vec![
                    json!(replacement.as_ref().and_then(|r| created.get(r)).copied().unwrap_or(ctx.anchor)),
                    json!(sid),
                    json!(w.users[&starter]),
                    json!(replacement),
                    json!(id),
                ],
            );
        } else if let Some(reason) = &n.stale {
            let by = n.stale_by.as_ref().map(|b| w.cast.session(b)).transpose()?;
            let at = by.map_or(ctx.anchor - 86_400_000, |b| {
                ctx.at(&b.start) + b.took.map_or(0, |t| t.0 * 500)
            });
            d1.exec(
                "UPDATE memories SET stale_at = ?, stale_reason = ?, stale_session_id = ?, stale_by = ? WHERE id = ?",
                vec![
                    json!(at),
                    json!(reason),
                    json!(by.map(|b| w.sessions[&b.id].id.clone())),
                    json!(by.map(|b| w.users[&b.by].clone())),
                    json!(id),
                ],
            );
        }
    }
    let retros = d1.query(
        "SELECT id, session_id, created_at, updated_at FROM retrospectives",
        &[],
    )?;
    for r in retros {
        let (Some(id), Some(sid)) = (
            r.get("id").and_then(Value::as_str),
            r.get("session_id").and_then(Value::as_str),
        ) else {
            continue;
        };
        let Some(c) = clocks.get(sid) else { continue };
        let at = |k: &str| r.get(k).and_then(Value::as_i64).map(|t| c.map(t));
        d1.exec(
            "UPDATE retrospectives SET created_at = ?, updated_at = ? WHERE id = ?",
            vec![json!(at("created_at")), json!(at("updated_at")), json!(id)],
        );
    }
    let published = d1.query(
        "SELECT id, session_id, created_at, expires_at FROM published_files",
        &[],
    )?;
    for r in published {
        let (Some(id), Some(sid)) = (
            r.get("id").and_then(Value::as_str),
            r.get("session_id").and_then(Value::as_str),
        ) else {
            continue;
        };
        let Some(c) = clocks.get(sid) else { continue };
        let at = |k: &str| r.get(k).and_then(Value::as_i64).map(|t| c.map(t));
        d1.exec(
            "UPDATE published_files SET created_at = ?, expires_at = ? WHERE id = ?",
            vec![json!(at("created_at")), json!(at("expires_at")), json!(id)],
        );
    }
    Ok(())
}

fn common_prefix(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

/// Schedules: their runs at the story's times, and the runs that started
/// nothing.
fn schedules(ctx: &Ctx, d1: &mut Db) -> Result<()> {
    let w = ctx.w;
    for s in &w.cast.schedules {
        let id = &w.schedules[&(s.project.clone(), s.name.clone())];
        let created = ctx.at(&s.created);
        let mut last = None;
        for spec in w.cast.sessions.iter().filter(|x| {
            x.schedule.as_deref() == Some(s.name.as_str())
                && cast::session_ref(&x.id).is_ok_and(|(p, _)| p == s.project)
        }) {
            let at = ctx.at(&spec.start);
            d1.exec(
                "UPDATE schedule_runs SET due_at = ?, at = ?, trigger = 'time', user_id = ? WHERE schedule_id = ? AND session_id = ?",
                vec![json!(at), json!(at), json!(w.users[&s.owner]), json!(id), json!(w.sessions[&spec.id].id)],
            );
            last = last.max(Some(at));
        }
        if let Some(skip) = &s.skipped {
            let at = ctx.at(&skip.at);
            // `{resets}`: when the owner's plan comes off its limit.
            let resets = w
                .cast
                .person(&s.owner)?
                .claude
                .iter()
                .find_map(|c| c.usage.and_then(|u| u.limited))
                .map_or(ctx.anchor + 3_600_000, |left| ctx.anchor + left.0 * 1000);
            let reason = skip.reason.replace("{resets}", &ctx.story.stamp(resets));
            d1.exec(
                "INSERT INTO schedule_runs (id, schedule_id, due_at, at, outcome, reason, trigger, user_id, session_id) VALUES (?, ?, ?, ?, 'skipped', ?, 'time', ?, NULL)",
                vec![json!(format!("srn_demo{}", id.trim_start_matches("sch_"))), json!(id), json!(at), json!(at), json!(reason), json!(w.users[&s.owner])],
            );
            last = last.max(Some(at));
        }
        // In the story's zone, on the days its Tuesday moved to.
        let cron = s.cron.as_deref().map(|c| ctx.story.cron(c)).transpose()?;
        let next = match (&s.cron, s.paused) {
            (Some(c), false) => ctx.story.next_run(c)?,
            _ => None,
        };
        d1.exec(
            "UPDATE schedules SET created_at = ?, updated_at = ?, last_run_at = COALESCE(?, last_run_at), timezone = ?, \
             cron = COALESCE(?, cron), next_run_at = COALESCE(?, next_run_at) WHERE id = ?",
            vec![
                json!(created),
                json!(last.unwrap_or(created)),
                json!(last),
                json!(ctx.story.zone),
                json!(cron),
                json!(next),
                json!(id),
            ],
        );
    }
    Ok(())
}

/// People, workspaces, projects and what connects them: when each came.
fn people_and_places(ctx: &Ctx, d1: &mut Db) -> Result<()> {
    let w = ctx.w;
    let earliest = w
        .cast
        .people
        .iter()
        .map(|p| ctx.at(&p.joined))
        .min()
        .unwrap_or(ctx.anchor)
        - 86_400_000;
    d1.exec(
        "UPDATE users SET created_at = ?, updated_at = ?, last_login_at = ? WHERE email = ?",
        vec![
            json!(earliest),
            json!(earliest),
            json!(earliest),
            json!(w.cast.instance.owner.email),
        ],
    );
    // The last session each person started is when they were last in.
    let mut last_in: BTreeMap<&str, i64> = BTreeMap::new();
    for s in &w.cast.sessions {
        let at = ctx.at(&s.start);
        let e = last_in.entry(s.by.as_str()).or_insert(at);
        *e = (*e).max(at);
    }
    for p in &w.cast.people {
        let joined = ctx.at(&p.joined);
        let uid = &w.users[&p.id];
        let seen = last_in
            .get(p.id.as_str())
            .copied()
            .unwrap_or(ctx.anchor - 26 * 3_600_000);
        d1.exec(
            "UPDATE users SET created_at = ?, updated_at = ?, last_login_at = ? WHERE id = ?",
            vec![json!(joined), json!(joined), json!(seen), json!(uid)],
        );
        d1.exec(
            "UPDATE user_emails SET created_at = ? WHERE user_id = ?",
            vec![json!(joined), json!(uid)],
        );
        d1.exec(
            "UPDATE tokens SET created_at = ?, last_used_at = ? WHERE user_id = ?",
            vec![json!(joined + 3_600_000), json!(seen), json!(uid)],
        );
        for (i, c) in p.claude.iter().enumerate() {
            let cred = &w.credentials[&(p.id.clone(), c.label.clone())];
            d1.exec(
                "UPDATE claude_credentials SET subscription_type = ?, created_at = ?, updated_at = ?, last_used_at = ? WHERE id = ?",
                vec![json!(c.plan), json!(joined + 7_200_000 + i as i64 * 86_400_000), json!(joined), json!(seen), json!(cred)],
            );
            d1.exec(
                "DELETE FROM claude_usage WHERE credential_id = ?",
                vec![json!(cred)],
            );
            if let Some(u) = c.usage {
                let five = 5 * 3_600_000;
                let five_resets = match u.limited {
                    Some(left) => ctx.anchor + left.0 * 1000,
                    None => ctx.anchor - ctx.anchor.rem_euclid(five) + five,
                };
                let week = 7 * 86_400_000;
                let week_resets =
                    ctx.anchor - (ctx.anchor - 3 * 86_400_000).rem_euclid(week) + week;
                for (window, used, resets, status) in [
                    (
                        "five_hour",
                        u.five_hour,
                        five_resets,
                        if u.limited.is_some() {
                            "rejected"
                        } else {
                            "allowed"
                        },
                    ),
                    ("seven_day", u.seven_day, week_resets, "allowed"),
                ] {
                    d1.exec(
                        "INSERT INTO claude_usage (credential_id, window, utilization, resets_at, status, observed_at) VALUES (?, ?, ?, ?, ?, ?)",
                        vec![json!(cred), json!(window), json!(used), json!(resets), json!(status), json!(ctx.anchor - 90_000)],
                    );
                }
            }
        }
    }
    for ws in &w.cast.workspaces {
        let wid = &w.workspaces[&ws.slug];
        let created = ctx.at(&ws.created);
        d1.exec(
            "UPDATE workspaces SET created_at = ?, updated_at = ?, avatar_updated_at = CASE WHEN avatar_updated_at IS NULL THEN NULL ELSE ? END WHERE id = ?",
            vec![json!(created), json!(created), json!(created + 3_600_000), json!(wid)],
        );
        for person in ws.people() {
            let joined = ws
                .joined
                .get(person)
                .map(|j| ctx.at(j))
                .unwrap_or_else(|| {
                    ctx.at(&w
                        .cast
                        .person(person)
                        .map(|p| p.joined.clone())
                        .unwrap_or(ws.created.clone()))
                })
                .max(created);
            d1.exec(
                "UPDATE workspace_members SET created_at = ? WHERE workspace_id = ? AND user_id = ?",
                vec![json!(joined), json!(wid), json!(w.users[person])],
            );
        }
        for inv in &ws.invitations {
            let at = ctx.at(&inv.at);
            d1.exec(
                "UPDATE workspace_invitations SET created_at = ?, expires_at = ? WHERE workspace_id = ? AND email = ?",
                vec![json!(at), json!(at + 14 * 86_400_000), json!(wid), json!(inv.email)],
            );
        }
        d1.exec(
            "UPDATE workspace_github SET created_at = ?, updated_at = ? WHERE workspace_id = ?",
            vec![
                json!(created + 3_600_000),
                json!(created + 3_600_000),
                json!(wid),
            ],
        );
        if let Some(l) = &ws.linear {
            let at = ctx.at(&l.at);
            d1.exec(
                "UPDATE workspace_linear SET created_at = ? WHERE workspace_id = ?",
                vec![json!(at), json!(wid)],
            );
            d1.exec(
                "UPDATE linear_installations SET created_at = ?, updated_at = ? WHERE organization_id = ?",
                vec![json!(at), json!(at), json!(format!("lin-{}", ws.slug))],
            );
            d1.exec(
                "UPDATE linear_accounts SET linked_at = ? WHERE organization_id = ?",
                vec![json!(at), json!(format!("lin-{}", ws.slug))],
            );
        }
    }
    for p in &w.cast.projects {
        let pid = &w.projects[&p.slug];
        let created = ctx.at(&p.created);
        let active = w
            .cast
            .sessions
            .iter()
            .filter(|s| cast::session_ref(&s.id).is_ok_and(|(x, _)| x == p.slug))
            .map(|s| ctx.at(&s.start))
            .max()
            .unwrap_or(created);
        d1.exec(
            "UPDATE projects SET created_at = ?, updated_at = ? WHERE id = ?",
            vec![json!(created), json!(active), json!(pid)],
        );
        for g in &p.grants {
            let (o, n) = g.repo.split_once('/').context("owner/name")?;
            d1.exec(
                "UPDATE project_repos SET created_at = ? WHERE project_id = ? AND owner = ? AND name = ?",
                vec![json!(ctx.at(&g.at)), json!(pid), json!(o), json!(n)],
            );
        }
        if let Some(r) = &p.reviews {
            d1.exec(
                "UPDATE project_reviews SET updated_at = ? WHERE project_id = ?",
                vec![json!(ctx.at(&r.since)), json!(pid)],
            );
        }
        d1.exec(
            "UPDATE project_members SET created_at = ? WHERE project_id = ?",
            vec![json!(created + 86_400_000), json!(pid)],
        );
    }
    Ok(())
}

/// Runners: the machines the story has, and when they came and were seen;
/// the one draining since the story has it.
fn runners(ctx: &Ctx, state: &std::path::Path, d1: &mut Db) -> Result<()> {
    let w = ctx.w;
    let template = d1
        .query(
            "SELECT info FROM runners WHERE info IS NOT NULL LIMIT 1",
            &[],
        )?
        .first()
        .and_then(|r| r.get("info"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .context("no runner has said hello")?;
    let fleet_path = sqlite::objects(state, "FleetObject")?
        .into_iter()
        .next()
        .context("no fleet object")?;
    let mut fleet = Db::open(&fleet_path);
    for r in &w.cast.runners {
        let id = &w.runners[&r.name];
        let info = d1
            .query("SELECT info FROM runners WHERE id = ?", &[json!(id)])?
            .first()
            .and_then(|x| x.get("info"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| template.clone());
        // The runner that never connected has no hello: the story's machine
        // from another's. The others get theirs once their hosts are back
        // ([`machines`]).
        let hello = machine(r, &info)?;
        let seen = r.seen.as_ref().map_or(ctx.anchor, |s| ctx.at(s));
        d1.exec(
            "UPDATE runners SET info = ?, created_at = ?, last_seen_at = ? WHERE id = ?",
            vec![
                json!(hello.to_string()),
                json!(ctx.at(&r.created)),
                json!(seen),
                json!(id),
            ],
        );
        d1.exec(
            "UPDATE runner_workspaces SET created_at = ? WHERE runner_id = ?",
            vec![json!(ctx.at(&r.created)), json!(id)],
        );
        if r.seen.is_some() {
            fleet.exec(
                "UPDATE runners SET last_seen = ? WHERE id = ?",
                vec![json!(seen), json!(id)],
            );
        }
        if let Some(d) = &r.drained {
            fleet.exec(
                "UPDATE runners SET draining_since = ?, drained_by = ? WHERE id = ?",
                vec![
                    json!(ctx.at(&d.at)),
                    json!(ctx.user(&d.by).to_string()),
                    json!(id),
                ],
            );
        }
    }
    fleet.commit()?;
    Ok(())
}

/// A runner's hello as the story's machine would say it, from what its
/// host (or another runner's, for one that never connected) said.
fn machine(r: &cast::RunnerSpec, info: &str) -> Result<Value> {
    let mut hello: Value = serde_json::from_str(info)?;
    hello["hostname"] = json!(r.name);
    hello["os"] = json!(r.os);
    hello["arch"] = json!(r.arch);
    hello["cpus"] = json!(r.cpus);
    hello["memory_mb"] = json!(r.memory_gb * 1024);
    hello["driver"] = json!(r.driver);
    let isolation = match r.driver.as_str() {
        "docker" => "container",
        "process" => "none",
        _ => "vm",
    };
    hello["drivers"] = json!([{ "name": r.driver, "available": true, "isolation": isolation }]);
    hello["capacity"] = json!(r.capacity);
    hello["sessions"] = json!([]);
    hello["auto_update"] = json!(true);
    let most =
        json!({ "cpus": (r.cpus / 2).max(2), "memory_mb": (r.memory_gb * 1024 / 2).max(4096) });
    hello["sandbox"] = json!({ "default": { "cpus": 4, "memory_mb": 8192 }, "max": most });
    serde_json::from_value::<fctl_proto::runner::HostHello>(hello.clone())
        .with_context(|| format!("runner {}'s machine doesn't read as a host's hello", r.name))?;
    Ok(hello)
}

/// The runners' machines, once their hosts are back: a host's hello, as it
/// reconnects, says what the machine it runs on is, which the story's
/// replace until it reconnects again.
pub fn machines(w: &World) -> Result<()> {
    let rows = super::wrangler_query(&w.root, &w.state, "SELECT id, info FROM runners")?;
    let info_of = |id: &str| {
        rows.iter()
            .find(|r| r["id"].as_str() == Some(id))
            .and_then(|r| r["info"].as_str())
            .map(str::to_owned)
    };
    let template = rows
        .iter()
        .find_map(|r| r["info"].as_str())
        .map(str::to_owned)
        .context("no runner has said hello")?;
    let mut sql = String::new();
    for r in &w.cast.runners {
        let id = &w.runners[&r.name];
        let hello = machine(r, &info_of(id).unwrap_or_else(|| template.clone()))?;
        sql.push_str(&format!(
            "UPDATE runners SET info = '{}' WHERE id = '{id}';\n",
            hello.to_string().replace('\'', "''")
        ));
    }
    super::wrangler_sql(&w.root, &w.state, &sql)
}

/// A session id of fctl's shape (`ses_` and a ULID) for a session that
/// started at `at`, the same for the same `seed`.
fn session_id(at: i64, seed: &str) -> String {
    use sha2::{Digest, Sha256};
    const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let rand = Sha256::digest(seed.as_bytes());
    let mut value: u128 = ((at.max(0) as u128) & ((1u128 << 48) - 1)) << 80;
    for (i, b) in rand.iter().take(10).enumerate() {
        value |= u128::from(*b) << (8 * (9 - i));
    }
    let mut out = [0u8; 26];
    for c in out.iter_mut().rev() {
        *c = CROCKFORD[(value & 0x1f) as usize];
        value >>= 5;
    }
    format!("ses_{}", String::from_utf8_lossy(&out))
}

/// The sessions before the cast's, which a project's header counts and its
/// list goes on with: rows alone, titled from the project's `history`,
/// started by people who were in its workspace by then, on runners it had
/// then, with the retrospectives its tab counts beyond the cast's.
fn fillers(ctx: &Ctx, d1: &mut Db) -> Result<()> {
    let w = ctx.w;
    for p in &w.cast.projects {
        let (Some(counted), false) = (p.counted, p.history.is_empty()) else {
            continue;
        };
        let own: Vec<&SessionSpec> = w
            .cast
            .sessions
            .iter()
            .filter(|s| cast::session_ref(&s.id).is_ok_and(|(x, _)| x == p.slug))
            .collect();
        let first = own
            .iter()
            .filter_map(|s| cast::session_ref(&s.id).ok().map(|(_, n)| n))
            .min()
            .unwrap_or(1);
        let listed = own.iter().filter(|s| s.review.is_none()).count() as u32;
        let wanted = counted.saturating_sub(listed);
        let free = first.saturating_sub(1);
        if wanted == 0 || free == 0 {
            continue;
        }
        // Spread over the numbers before the cast's: those between go to
        // reviews, which no list shows.
        let wanted = wanted.min(free);
        let numbers: Vec<u32> = (1..=wanted)
            .map(|k| (u64::from(k) * u64::from(free) / u64::from(wanted)) as u32)
            .collect();
        let count = numbers.len();
        let ws = w.cast.workspace(&p.workspace)?;
        let created = ctx.at(&ws.created);
        let people: Vec<(&str, i64)> = ws
            .people()
            .into_iter()
            .map(|id| {
                let joined = ws
                    .joined
                    .get(id)
                    .or_else(|| w.cast.person(id).ok().map(|x| &x.joined))
                    .map_or(created, |j| ctx.at(j));
                (id, joined.max(created))
            })
            .collect();
        let runners: Vec<(&str, i64)> = w
            .cast
            .runners
            .iter()
            .filter(|r| r.workspaces.contains(&p.workspace) && !r.personal.contains(&p.workspace))
            .map(|r| (r.name.as_str(), ctx.at(&r.created)))
            .collect();
        let from = ctx.at(&p.created) + 86_400_000;
        let until = own
            .iter()
            .map(|s| ctx.at(&s.start))
            .min()
            .unwrap_or(ctx.anchor)
            - 3_600_000;
        let span = (until - from).max(1);
        let pid = &w.projects[&p.slug];
        // Pull requests count back from the cast's first, two apart where
        // there is room for other people's in between.
        let first_pr = own
            .iter()
            .filter_map(|s| s.pr.as_ref().map(|x| x.number))
            .min()
            .unwrap_or(100);
        let with_pr: Vec<usize> = (0..count)
            .filter(|i| p.repo.is_some() && i % 9 != 4)
            .collect();
        let step = ((first_pr - 1) / (with_pr.len() as u64).max(1)).clamp(1, 2);
        let pr_of: BTreeMap<usize, u64> = with_pr
            .iter()
            .rev()
            .enumerate()
            .filter_map(|(k, i)| {
                let n = first_pr.checked_sub((k as u64 + 1) * step)?;
                (n >= 1).then_some((*i, n))
            })
            .collect();
        let outcomes = filler_outcomes(ctx, p, d1, count)?;
        for (i, n) in numbers.iter().enumerate() {
            // The newest take the history's last titles.
            let back = count - 1 - i;
            let title = &p.history[p.history.len() - 1 - back % p.history.len()];
            // Spread over the project's life, a little irregular, in order.
            let jitter = (i64::from(*n) * 7919 % 97) * 60_000;
            let at = from + span * i as i64 / count as i64 + jitter.min(span / count as i64 / 2);
            let there: Vec<&str> = people
                .iter()
                .filter(|(_, joined)| *joined <= at)
                .map(|(id, _)| *id)
                .collect();
            let by = there
                .get((i * 7 + i / 3) % there.len().max(1))
                .copied()
                .unwrap_or(ws.owner.as_str());
            let took = (12 + (i * 37) % 70) as i64 * 60_000;
            let on: Vec<&str> = runners
                .iter()
                .filter(|(_, made)| *made <= at)
                .map(|(name, _)| *name)
                .collect();
            let runner = on.get(i % on.len().max(1)).map(|r| w.runners[*r].clone());
            let pr = pr_of.get(&i).copied();
            let branch = format!("fctl/{n}-{}", super::world::slug(title));
            let stats = json!({
                "turns": 1 + i % 3, "tool_calls": 14 + (i * 13) % 60, "files_changed": 1 + i % 7,
                "additions": 8 + (i * 31) % 240, "deletions": (i * 17) % 60, "busy_ms": took / 2,
                "input_tokens": 2_000 + (i * 977) % 9_000, "output_tokens": 9_000 + (i * 3_331) % 40_000,
                "cache_read_tokens": 400_000 + (i * 91_771) % 2_400_000,
            });
            let (url, state, prs) = match (&p.repo, pr) {
                (Some(repo), Some(pr)) => {
                    let (owner, name) = repo.split_once('/').context("owner/name")?;
                    let url = format!("https://github.com/{repo}/pull/{pr}");
                    let state = if i % 23 == 11 { "closed" } else { "merged" };
                    let list = json!([{ "number": pr, "url": url, "title": title, "state": state,
                        "repo": { "owner": owner, "name": name }, "state_changed_at": at + took + 3_600_000, "updated_at": at + took + 3_600_000 }]);
                    (Some(url), Some(state), Some(list.to_string()))
                }
                _ => (None, None, None),
            };
            let id = session_id(at, &format!("{}#{n}", p.slug));
            d1.exec(
                "INSERT INTO sessions (id, project_id, number, title, prompt, status, created_by, runner_id, branch, base_branch, model, \
                 claude_session_id, pr_number, pr_url, pr_title, pr_state, pull_requests, stats, archived, created_at, updated_at, \
                 started_at, ended_at, last_activity_at, search_indexed) \
                 VALUES (?, ?, ?, ?, ?, 'stopped', ?, ?, ?, 'main', ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?, 1)",
                vec![
                    json!(id),
                    json!(pid),
                    json!(n),
                    json!(title),
                    json!(title),
                    json!(w.users[by]),
                    json!(runner),
                    json!(branch),
                    json!(p.model.clone().unwrap_or_else(|| "opus".into())),
                    json!(format!("demo-{}-{n}", p.slug)),
                    json!(pr),
                    json!(url),
                    json!(pr.map(|_| title)),
                    json!(state),
                    json!(prs),
                    json!(stats.to_string()),
                    json!(at),
                    json!(at + took),
                    json!(at + 20_000),
                    json!(at + took),
                    json!(at + took),
                ],
            );
            if let Some(outcome) = outcomes.get(back).copied().flatten() {
                let repo = p.repo.as_deref().map(|r| r.rsplit('/').next().unwrap_or(r));
                let worked_on = match (repo, pr) {
                    (Some(r), Some(n)) => format!("{title} ({r}#{n})."),
                    _ => format!("{title}."),
                };
                let challenges = match outcome {
                    "partial" => Some(
                        [
                            "What's left is listed in the pull request.",
                            "The rest needs a decision, which the pull request asks for.",
                            "Stopped where the tests need a device the sandbox doesn't have.",
                        ][i % 3],
                    ),
                    "blocked" => Some(
                        [
                            "It needs access the sandbox doesn't have.",
                            "It waits for the project's owner to choose a way.",
                        ][i % 2],
                    ),
                    _ => None,
                };
                d1.exec(
                    "INSERT INTO retrospectives (id, project_id, session_id, created_at, outcome, worked_on, challenges) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    vec![
                        json!(session_id(at + took, &format!("retro {}#{n}", p.slug)).replacen("ses_", "rtr_", 1)),
                        json!(pid),
                        json!(id),
                        json!(at + took),
                        json!(outcome),
                        json!(worked_on),
                        json!(challenges),
                    ],
                );
            }
        }
    }
    Ok(())
}

/// The outcomes of the retrospectives of a project's `count` older
/// sessions, newest first: what its `retros` count beyond those its
/// cast's sessions recorded, spread out; the oldest sessions have none.
fn filler_outcomes(
    ctx: &Ctx,
    p: &cast::Project,
    d1: &mut Db,
    count: usize,
) -> Result<Vec<Option<&'static str>>> {
    let [completed, partial, blocked] = p.retros[..] else {
        return Ok(Vec::new());
    };
    let recorded = d1.query(
        "SELECT outcome, COUNT(*) AS n FROM retrospectives WHERE project_id = ? GROUP BY outcome",
        &[json!(ctx.w.projects[&p.slug])],
    )?;
    let had = |outcome: &str| {
        recorded
            .iter()
            .find(|r| r.get("outcome").and_then(Value::as_str) == Some(outcome))
            .and_then(|r| r.get("n"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize
    };
    let left = |total: u32, outcome: &str| (total as usize).saturating_sub(had(outcome));
    let (partial, blocked) = (left(partial, "partial"), left(blocked, "blocked"));
    let total = (left(completed, "completed") + partial + blocked).min(count);
    let mut out = vec![Some("completed"); total];
    out.resize(count, None);
    for (k, outcome, phase) in [(partial, "partial", 0.5), (blocked, "blocked", 0.3)] {
        for j in 0..k {
            let mut at = ((j as f64 + phase) * total as f64 / k as f64) as usize;
            while at < total && out[at] != Some("completed") {
                at += 1;
            }
            if let Some(slot) = out.get_mut(at) {
                *slot = Some(outcome);
            }
        }
    }
    Ok(out)
}

/// The audit log: each entry when the story has what it records.
fn audit(ctx: &Ctx, clocks: &BTreeMap<String, Clock>, d1: &mut Db) -> Result<()> {
    let w = ctx.w;
    let rows = d1.query("SELECT id, ts, action, target FROM audit_log", &[])?;
    let mut when: BTreeMap<&str, i64> = BTreeMap::new();
    for ws in &w.cast.workspaces {
        when.insert(w.workspaces[&ws.slug].as_str(), ctx.at(&ws.created));
    }
    for p in &w.cast.projects {
        when.insert(w.projects[&p.slug].as_str(), ctx.at(&p.created));
    }
    for r in &w.cast.runners {
        when.insert(
            w.runners[&r.name].as_str(),
            r.drained
                .as_ref()
                .map_or(ctx.at(&r.created), |d| ctx.at(&d.at)),
        );
    }
    for p in &w.cast.people {
        when.insert(w.users[&p.id].as_str(), ctx.at(&p.joined));
    }
    for r in rows {
        let (Some(id), Some(ts)) = (
            r.get("id").and_then(Value::as_str),
            r.get("ts").and_then(Value::as_i64),
        ) else {
            continue;
        };
        let target = r.get("target").and_then(Value::as_str).unwrap_or_default();
        let new = clocks
            .get(target)
            .map(|c| c.map(ts))
            .or_else(|| when.get(target).copied())
            .unwrap_or(ctx.anchor - 30 * 86_400_000 + (ts - w.started).max(0) * 1000);
        d1.exec(
            "UPDATE audit_log SET ts = ? WHERE id = ?",
            vec![json!(new), json!(id)],
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Clock, remap};
    use serde_json::json;

    #[test]
    fn clocks_stretch_between_their_points_and_move_beyond_them() {
        let c = Clock::new(vec![(1000, 100_000), (2000, 200_000), (3000, 230_000)]);
        assert_eq!(c.map(1000), 100_000);
        assert_eq!(c.map(1500), 150_000);
        assert_eq!(c.map(2500), 215_000);
        assert_eq!(c.map(4000), 231_000);
        assert_eq!(c.map(900), 99_900);
        // A point out of order is left out.
        let c = Clock::new(vec![(1000, 100_000), (2000, 90_000), (3000, 300_000)]);
        assert_eq!(c.map(2000), 200_000);
    }

    #[test]
    fn times_move_wherever_they_are() {
        let c = Clock::new(vec![(1_700_000_000_000, 1_600_000_000_000)]);
        let mut v = json!({"created_at": 1_700_000_000_500_i64, "stats": {"busy_ms": 840_000, "turn_started_at": 1_700_000_001_000_i64},
            "limit": {"resetsAt": 1_700_000_100_i64}, "tokens": 12});
        remap(&mut v, &c, (1_699_999_000_000, 1_700_001_000_000));
        assert_eq!(v["created_at"], 1_600_000_000_500_i64);
        assert_eq!(v["stats"]["turn_started_at"], 1_600_000_001_000_i64);
        assert_eq!(v["stats"]["busy_ms"], 840_000);
        assert_eq!(v["limit"]["resetsAt"], 1_600_000_100_i64);
        assert_eq!(v["tokens"], 12);
    }
}
