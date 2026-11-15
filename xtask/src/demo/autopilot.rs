//! The cast's autopilots (experimental, `[[autopilot]]`): each put on
//! through the API, as its owner would, then woken by the control plane in
//! sessions of its own, which play their transcripts, as the sessions it
//! starts play theirs. The seed starts none of them. What people do to it
//! meanwhile (notes, answers, secrets), and what moves its story on, are its
//! beats, which the seed does in order, each once fake-claude's replay log
//! has the lines the one before it waits for (`then`).
//!
//! A crew session's first turn waits for the seed's go-ahead (its first
//! step looks for `<transcript>-<turn>` in [`SIGNALS`]), so that its turn
//! ends, and wakes the autopilot a minute later, where the story has it:
//! alone, or with another's for one wake.

use super::cast::{AutopilotSpec, Beat, Project};
use super::world::{Made, World, transcript_of};
use super::{now_ms, wrangler_sql};
use crate::dev::{self, Auth, Children};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Where, in the demo's state, crew sessions' waiting turns look for the
/// seed's go-ahead, which the replays find as `$DEMO_SIGNALS`.
pub const SIGNALS: &str = "signals";

/// How much older a session becomes for its autopilot's next wake to go to
/// a fresh one: more than `fctl_domain::autopilot::SHIFT_MS`.
const AGED_MS: i64 = 13 * 3_600_000;

impl World<'_> {
    /// Plays each autopilot's story, after the people's sessions.
    pub fn autopilots(&mut self, children: &mut Children) -> Result<()> {
        let cast = self.cast;
        for a in &cast.autopilots {
            self.autopilot(a, children)
                .with_context(|| format!("{}'s autopilot", a.project))?;
        }
        Ok(())
    }

    fn autopilot(&mut self, a: &AutopilotSpec, children: &mut Children) -> Result<()> {
        let cast = self.cast;
        let p = cast.project(&a.project)?;
        let pid = self.projects[&p.slug].clone();
        let wid = self.workspaces[&p.workspace].clone();
        std::fs::create_dir_all(self.state.join(SIGNALS))?;
        // Putting a project on autopilot is a sensitive action.
        self.confirm(&a.by)?;
        // Its workspace's admins let its projects have autopilots.
        self.api(
            &a.by,
            "PATCH",
            &format!("/workspaces/{wid}"),
            Some(json!({ "autopilots": true })),
        )?;
        // Its sessions run on the story's runner: the workspace's others
        // drain while it runs, and take sessions again after.
        let drained = self.drain_others(&p.workspace, &a.runner)?;
        let body = json!({
            "direction": a.direction,
            "powers": a.powers,
            "check_in_minutes": a.check_in,
            "crew_max": a.crew,
            "model": a.model.as_deref().unwrap_or("default"),
            "effort": a.effort.as_deref().unwrap_or("default"),
        });
        let put = format!("/projects/{pid}/autopilot");
        self.api(&a.by, "PUT", &put, Some(body))?;
        self.beat_times.push((now_ms(), a.at.clone()));
        self.waits(&a.then, children)?;
        self.found(p, &pid, &a.by)?;
        for (i, b) in a.beats.iter().enumerate() {
            self.beat(p, &pid, b, children)
                .with_context(|| format!("its beat {}", i + 1))?;
            self.found(p, &pid, &a.by)?;
        }
        for (id, owner) in drained {
            self.api(
                &owner,
                "PATCH",
                &format!("/runners/{id}"),
                Some(json!({ "draining": false })),
            )?;
        }
        Ok(())
    }

    fn beat(&mut self, p: &Project, pid: &str, b: &Beat, children: &mut Children) -> Result<()> {
        if b.handoff {
            let a = self.api("noor", "GET", &format!("/projects/{pid}/autopilot"), None)?;
            let session = a["session"]["id"]
                .as_str()
                .context("it has no session of its own to hand off from")?;
            wrangler_sql(
                &self.root,
                &self.state,
                &format!(
                    "UPDATE sessions SET created_at = created_at - {AGED_MS} WHERE id = '{session}';"
                ),
            )?;
        }
        let when = now_ms();
        let by = b.by.as_deref().unwrap_or("noor");
        if let Some(text) = &b.note {
            self.api(
                by,
                "POST",
                &format!("/projects/{pid}/autopilot/notes"),
                Some(json!({ "text": text })),
            )?;
        }
        if let Some(name) = &b.secret {
            let ask = self.open_ask(pid, by, |a| a["secret"]["name"] == name.as_str())?;
            let value = format!("demo-{}-{}", p.slug, name.to_lowercase());
            self.api(
                by,
                "POST",
                &format!("/projects/{pid}/autopilot/asks/{ask}/answer"),
                Some(json!({ "secret": value })),
            )?;
        }
        if let (Some(start), Some(text)) = (&b.answer, &b.text) {
            let ask = self.open_ask(pid, by, |a| {
                a["kind"] == "question" && a["text"].as_str().is_some_and(|t| t.starts_with(start))
            })?;
            self.api(
                by,
                "POST",
                &format!("/projects/{pid}/autopilot/asks/{ask}/answer"),
                Some(json!({ "answer": text })),
            )?;
        }
        for id in &b.release {
            self.release(id)?;
        }
        for id in &b.merge {
            self.merge(p, id)?;
        }
        for id in &b.stop {
            let made = self.made(id)?;
            let s = self.cast.session(id)?;
            self.api(&s.by, "POST", &format!("/sessions/{}/stop", made.id), None)?;
            self.stopped.insert(id.clone());
        }
        if b.wake {
            self.wake_asked(pid)?;
        }
        if let Some(at) = &b.at {
            self.beat_times.push((when, at.clone()));
        }
        self.waits(&b.then, children)
    }

    /// The id of the autopilot's open ask that `pick` picks.
    fn open_ask(&self, pid: &str, who: &str, pick: impl Fn(&Value) -> bool) -> Result<String> {
        let a = self.api(who, "GET", &format!("/projects/{pid}/autopilot"), None)?;
        a["asks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|x| x["status"] == "open")
            .find(|x| pick(x))
            .and_then(|x| x["id"].as_str())
            .map(str::to_owned)
            .with_context(|| format!("no open ask of the kind the story answers: {}", a["asks"]))
    }

    /// Lets the waiting turn of session `id` (`project#number`) go on.
    fn release(&mut self, id: &str) -> Result<()> {
        let s = self.cast.session(id)?;
        let turn = 1 + self.releases.keys().filter(|(x, _)| x == id).count();
        let signal = self
            .state
            .join(SIGNALS)
            .join(format!("{}-{turn}", transcript_of(s)));
        std::fs::write(&signal, "go")?;
        self.releases.insert((id.to_owned(), turn), now_ms());
        Ok(())
    }

    /// Merges session `id`'s branch into main on the stand-in, as its pull
    /// request's squash merge, for the sessions after it.
    fn merge(&self, p: &Project, id: &str) -> Result<()> {
        let s = self.cast.session(id)?;
        let repo = p
            .repo
            .as_deref()
            .context("a project without a repository")?;
        let pr = s.pr.as_ref().context("a session without a pull request")?;
        let branch = self
            .made(id)?
            .branch
            .clone()
            .with_context(|| format!("{id} has no branch"))?;
        let title = pr.title.as_deref().or(s.title.as_deref()).unwrap_or(id);
        let app = &self.cast.instance.github_app;
        let author = (
            format!("{app}[bot]"),
            format!("{app}[bot]@users.noreply.github.com"),
        );
        self.github
            .merge(repo, &branch, (title, pr.number), (&author.0, &author.1))
    }

    /// Makes the autopilot's wake at the time it asked for due now, and has
    /// the alarm that wakes autopilots ring for it (any change to a
    /// schedule sets that alarm again).
    fn wake_asked(&self, pid: &str) -> Result<()> {
        let now = now_ms();
        wrangler_sql(
            &self.root,
            &self.state,
            &format!(
                "UPDATE autopilots SET wake_at = {now}, asked_wake_at = {now} WHERE project_id = '{pid}';"
            ),
        )?;
        let (sched, sid) = self
            .schedules
            .iter()
            .filter_map(|((project, name), sid)| {
                Some((self.cast.schedule(project, name).ok()?, sid))
            })
            .find(|(s, _)| s.cron.is_some())
            .context("a schedule to ring the alarm with")?;
        self.api(
            &sched.owner,
            "PATCH",
            &format!("/schedules/{sid}"),
            Some(json!({ "cron": sched.cron, "timezone": sched.zone })),
        )?;
        Ok(())
    }

    /// Drains the runners of `workspace` that take sessions, other than
    /// `keep`, as their people would: those it drained, with their owners.
    fn drain_others(&self, workspace: &str, keep: &str) -> Result<Vec<(String, String)>> {
        let mut drained = Vec::new();
        for r in &self.cast.runners {
            if r.name == keep
                || r.seen.is_some()
                || r.drained.is_some()
                || !r.workspaces.iter().any(|w| w == workspace)
            {
                continue;
            }
            let id = self.runners[&r.name].clone();
            self.api(
                &r.owner,
                "PATCH",
                &format!("/runners/{id}"),
                Some(json!({ "draining": true })),
            )?;
            drained.push((id, r.owner.clone()));
        }
        Ok(drained)
    }

    /// Confirms it's `who` for the next 15 minutes, where the instance asks
    /// for it before sensitive actions: with a code from their authenticator
    /// app, which only Noor has.
    fn confirm(&self, who: &str) -> Result<()> {
        let auth = Auth::Bearer(self.tokens[who].clone());
        let (_, v, _) = dev::call("GET", "/auth/verify", Some(&auth), None)?;
        if v["required"] != true || v["verified"] == true {
            return Ok(());
        }
        let key = self
            .totp
            .as_ref()
            .filter(|_| who == "noor")
            .context("confirming it's them takes an authenticator app only Noor has")?;
        // A code works once: should this step's be used up, the next one's.
        for step in 0..3 {
            let code = fctl_domain::totp::code_text(
                key,
                fctl_domain::totp::step(now_ms()) + i64::from(step > 0),
            );
            let res = dev::call(
                "POST",
                "/auth/verify",
                Some(&auth),
                Some(json!({ "code": code })),
            )?;
            if (200..300).contains(&res.0) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_secs(31));
        }
        bail!("couldn't confirm it's {who}")
    }

    /// Waits for each line of fake-claude's replay log, `<name>: <what>`.
    fn waits(&self, lines: &[String], children: &mut Children) -> Result<()> {
        for line in lines {
            let (name, what) = line
                .split_once(": ")
                .with_context(|| format!("{line:?} isn't a replay log line"))?;
            self.until_logged(name, what, children)?;
        }
        Ok(())
    }

    /// The autopilot's sessions that the seed hasn't met yet, by their
    /// numbers: kept as the cast's, with the pull requests the stand-in
    /// answers for their branches.
    fn found(&mut self, p: &Project, pid: &str, who: &str) -> Result<()> {
        let list = self.api(
            who,
            "GET",
            &format!("/sessions?project={pid}&hidden=1&limit=100"),
            None,
        )?;
        let mut numbers: BTreeMap<u32, Value> = BTreeMap::new();
        for s in list["sessions"].as_array().into_iter().flatten() {
            if let Some(n) = s["number"].as_u64().and_then(|n| u32::try_from(n).ok()) {
                numbers.insert(n, s.clone());
            }
        }
        for (n, s) in numbers {
            let key = format!("{}#{n}", p.slug);
            if self.sessions.contains_key(&key) {
                continue;
            }
            let spec = self.cast.session(&key).with_context(|| {
                format!(
                    "the autopilot started {key} ({}), which the cast doesn't have",
                    s["title"]
                )
            })?;
            if spec.autopilot.is_none() {
                bail!("{key} is the autopilot's, but the cast doesn't say so");
            }
            let id = s["id"].as_str().context("a session without an id")?;
            let full = self.api(who, "GET", &format!("/sessions/{id}"), None)?;
            let branch = full["branch"].as_str().map(str::to_owned);
            if let (Some(pr), Some(repo), Some(b)) = (&spec.pr, &p.repo, &branch) {
                let title = pr
                    .title
                    .as_deref()
                    .or(spec.title.as_deref())
                    .unwrap_or(&key);
                self.github.pull(repo, pr.number, title, b)?;
            }
            self.numbers.insert(p.slug.clone(), n);
            self.sessions.insert(
                key,
                Made {
                    id: id.to_owned(),
                    branch,
                    messages: Vec::new(),
                    follow_ups: BTreeMap::new(),
                },
            );
        }
        Ok(())
    }

    fn made(&self, id: &str) -> Result<&Made> {
        self.sessions
            .get(id)
            .with_context(|| format!("{id} hasn't started"))
    }
}
