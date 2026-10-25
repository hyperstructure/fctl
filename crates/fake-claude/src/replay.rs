//! Replays: turns written down in advance, which fake-claude plays back as
//! Claude Code streams its own (`--replays <dir>`).
//!
//! Each `*.toml` file in the directory is one session's work. Its `match`
//! picks the session: the first message that contains it (any case) starts
//! the replay, and its first `[[turn]]` answers that message. Each later
//! turn answers the next message, or the next one that contains its own
//! `match`; a message no turn answers gets fake-claude's usual behavior.
//!
//! A turn is its `[[turn.step]]`s, played in order. Each step has one of
//! these keys, and the details beside it:
//!
//! | Key | Step |
//! |---|---|
//! | `think` | a thinking block |
//! | `say` | text, streamed |
//! | `bash` | a `Bash` call with a written `output` (and `description`, `error`) |
//! | `run` | a `Bash` call that runs for real in the working directory: its output is what it printed (`show` is the command people see, when it differs) |
//! | `read` | a `Read` of a file: its lines from the working directory, else `output` |
//! | `edit` | an `Edit` of a file, `old` to `new` (`replace_all`), made for real when the file has `old` |
//! | `write` | a `Write` of `content` to a file, made for real |
//! | `grep`, `glob` | a search with a written `output` (`path`, `glob`, `mode`) |
//! | `tool` | any other tool, with its `input` and `output` |
//! | `tools` | several of the calls above at once: one response asks for all of them |
//! | `todo` | a `TodoWrite` of `[content, status]` pairs |
//! | `agent` | a subagent (`Agent` tool) with this description, its `prompt`, its `steps` (calls and `say`s) and its `report` |
//! | `commit` | `git add -A && git commit` with this message, for real |
//! | `push` | `git push -u origin HEAD`, for real where it can, else as it would print |
//! | `pr` | `gh pr create` with this title (and `body`), which prints the pull request `number`'s URL on the repository's GitHub |
//! | `limit` | the five-hour usage limit, which resets this long after: the turn ends there |
//! | `usage` | plan usage: `{ five_hour = 0.34, seven_day = 0.58 }` |
//! | `pause` | waits this long |
//! | `silently` | a shell command run for real, which the transcript doesn't show |
//! | `context` | the tokens in context from here on |
//!
//! A call with `seconds` takes that long (its progress ticking), and one
//! with `hold = true` never ends: the session works on until someone
//! interrupts or stops it. A turn's `duration` is what its result says it
//! took, for the session's busy time, and its `result` what it says it did
//! (the last `say` when absent). Times are seconds, or `"1m30s"`.
//!
//! Beside the files, `speed` (a number) makes everything that many times
//! faster, and `env` (`KEY=value` lines) adds to the environment of the
//! commands steps run. Both are read again at every turn. What goes wrong
//! (a file that doesn't parse, an edit whose file lacks what it replaces, a
//! command that fails silently) goes to `replay.log` there, not to stderr,
//! which fctl-agent would show in the transcript.

use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

/// Where [`warn`] writes: `replay.log` beside the replays.
static LOG: OnceLock<PathBuf> = OnceLock::new();

/// Notes where a replay got to in `replay.log`, for whoever waits for it:
/// `<name>: holds` once it holds a call, `<name>: limited` once it hit the
/// usage limit, `<name>: turn <n> played` once a turn ended (and
/// [`Library::start`] notes `<name>: started`).
fn note(what: &str) {
    if LOG.get().is_some() {
        warn(what);
    }
}

/// Notes what went wrong with a replay in `replay.log` (stderr before a
/// library has loaded).
fn warn(what: &str) {
    let written = LOG
        .get()
        .is_some_and(|path| append_line(path, what).is_ok());
    if !written {
        eprintln!("fake-claude: {what}");
    }
}

/// Adds a line to a log in one write, so that lines that other sessions'
/// fake-claudes add at the same time don't run into it.
fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(format!("{line}\n").as_bytes())
}

/// The replays in a directory, and what it says about playing them.
pub struct Library {
    dir: PathBuf,
    replays: Vec<Replay>,
}

/// One session's turns.
#[derive(Clone, Debug)]
pub struct Replay {
    pub name: String,
    pub matches: String,
    pub turns: Vec<Turn>,
}

#[derive(Clone, Debug)]
pub struct Turn {
    /// The message it answers: the next one that contains this. Absent:
    /// the next message, whatever it says.
    pub matches: Option<String>,
    pub steps: Vec<Step>,
    /// What its result says it took.
    pub duration: Option<Duration>,
    /// What its result says it did.
    pub result: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Step {
    Think(String),
    Say(String),
    /// One call, or several at once.
    Calls(Vec<Call>),
    Agent(Agent),
    Limit(Duration),
    Usage {
        five_hour: f64,
        seven_day: f64,
    },
    Pause(Duration),
    Silently(String),
    Context(u64),
}

/// A tool call: what the transcript shows of it, and what happens for real.
#[derive(Clone, Debug)]
pub struct Call {
    pub name: String,
    pub input: Value,
    pub effect: Effect,
    /// What it printed, when written down (it overrides what a real effect
    /// printed).
    pub output: Option<String>,
    pub error: bool,
    /// How long it takes.
    pub seconds: Option<Duration>,
    /// It never ends.
    pub hold: bool,
}

#[derive(Clone, Debug)]
pub enum Effect {
    None,
    /// A shell command run for real.
    Run(String),
    Read(String),
    Edit {
        path: String,
        old: String,
        new: String,
        all: bool,
    },
    Write {
        path: String,
        content: String,
    },
    Commit(String),
    Push,
    PullRequest {
        number: u64,
    },
}

/// A subagent's work.
#[derive(Clone, Debug)]
pub struct Agent {
    pub description: String,
    pub prompt: String,
    /// Its calls, and what it says between them.
    pub steps: Vec<AgentStep>,
    pub report: String,
}

#[derive(Clone, Debug)]
pub enum AgentStep {
    Say(String),
    Call(Call),
}

/// A replay under way: the turns it has left.
pub struct Playing {
    pub replay: Replay,
    pub next: usize,
}

impl Library {
    /// Every replay in `dir`. A file that doesn't parse is left out, with
    /// what is wrong with it on stderr.
    pub fn load(dir: &Path) -> Self {
        let _ = LOG.set(dir.join("replay.log"));
        let mut replays = Vec::new();
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|d| {
                d.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                    .collect()
            })
            .unwrap_or_else(|e| {
                warn(&format!("reading replays in {}: {e}", dir.display()));
                Vec::new()
            });
        files.sort();
        for path in files {
            match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| parse(&text))
            {
                Ok(mut r) => {
                    r.name = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    replays.push(r);
                }
                Err(e) => warn(&format!("replay {}: {e}", path.display())),
            }
        }
        Self {
            dir: dir.to_owned(),
            replays,
        }
    }

    /// How many times faster than written to play: `speed` beside the
    /// files, else 1.
    pub fn speed(&self) -> f64 {
        std::fs::read_to_string(self.dir.join("speed"))
            .ok()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .filter(|s| s.is_finite() && *s > 0.0)
            .unwrap_or(1.0)
    }

    /// What `env` beside the files adds to the commands steps run.
    pub fn env(&self) -> Vec<(String, String)> {
        std::fs::read_to_string(self.dir.join("env"))
            .map(|text| parse_env(&text))
            .unwrap_or_default()
    }

    /// The replay a session's first message starts, if any: the first whose
    /// `match` it contains that no session has started yet, else the first
    /// whose `match` it contains. `replay.log` says which have started
    /// (`<name>: started`), so that two reviews of one pull request, whose
    /// messages differ only in the commit, play a replay each.
    pub fn start(&self, text: &str) -> Option<Playing> {
        let lower = text.to_lowercase();
        let log = self.dir.join("replay.log");
        let started = std::fs::read_to_string(&log).unwrap_or_default();
        let taken = |name: &str| {
            started
                .lines()
                .any(|l| l.strip_suffix(": started") == Some(name))
        };
        let matching: Vec<&Replay> = self
            .replays
            .iter()
            .filter(|r| lower.contains(&r.matches.to_lowercase()))
            .collect();
        let replay = matching
            .iter()
            .find(|r| !taken(&r.name))
            .or(matching.first())?;
        let _ = append_line(&log, &format!("{}: started", replay.name));
        Some(Playing {
            replay: (*replay).clone(),
            next: 0,
        })
    }
}

impl Playing {
    /// The turn that answers `text`, if the replay has one left for it: its
    /// next turn, or the next one whose `match` the text contains.
    pub fn take(&mut self, text: &str) -> Option<Turn> {
        let lower = text.to_lowercase();
        let rest = self.replay.turns.get(self.next..)?;
        let at = rest.iter().position(|t| {
            t.matches
                .as_ref()
                .is_none_or(|m| lower.contains(&m.to_lowercase()))
        })?;
        // Turns without a `match` answer only the very next message.
        let turn = rest.get(at)?;
        if at > 0 && turn.matches.is_none() {
            return None;
        }
        self.next += at + 1;
        Some(turn.clone())
    }
}

/// `KEY=value` lines; blank lines and `#` comments don't count.
fn parse_env(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.to_owned()))
        .collect()
}

/// A replay file.
pub fn parse(text: &str) -> Result<Replay, String> {
    let raw: RawReplay = toml::from_str(text).map_err(|e| e.to_string())?;
    if raw.matches.trim().is_empty() {
        return Err("`match` is empty".into());
    }
    let mut turns = Vec::new();
    for (i, t) in raw.turn.into_iter().enumerate() {
        let mut steps = Vec::new();
        for (j, s) in t.step.into_iter().enumerate() {
            steps.push(step(s).map_err(|e| format!("turn {}, step {}: {e}", i + 1, j + 1))?);
        }
        turns.push(Turn {
            matches: t.matches.filter(|m| !m.trim().is_empty()),
            steps,
            duration: t
                .duration
                .map(|d| d.parse())
                .transpose()
                .map_err(|e| format!("turn {}: {e}", i + 1))?,
            result: t.result,
        });
    }
    if turns.is_empty() {
        return Err("it has no `[[turn]]`".into());
    }
    Ok(Replay {
        name: String::new(),
        matches: raw.matches,
        turns,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReplay {
    #[serde(rename = "match")]
    matches: String,
    #[serde(default)]
    turn: Vec<RawTurn>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTurn {
    #[serde(rename = "match")]
    matches: Option<String>,
    duration: Option<Seconds>,
    result: Option<String>,
    #[serde(default)]
    step: Vec<RawStep>,
}

/// Seconds, or a duration such as `"1m30s"`.
#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum Seconds {
    Number(f64),
    Text(String),
}

impl Seconds {
    fn parse(&self) -> Result<Duration, String> {
        let secs = match self {
            Self::Number(n) => *n,
            Self::Text(t) => parse_duration(t)?,
        };
        if !secs.is_finite() || secs < 0.0 {
            return Err(format!("{secs} isn't a duration"));
        }
        Ok(Duration::from_secs_f64(secs))
    }
}

/// `"90"`, `"45s"`, `"1m30s"`, `"2h"`, `"1h5m"`: seconds.
pub fn parse_duration(text: &str) -> Result<f64, String> {
    let t = text.trim();
    if let Ok(n) = t.parse::<f64>() {
        return Ok(n);
    }
    let mut total = 0.0;
    let mut number = String::new();
    for c in t.chars() {
        if c.is_ascii_digit() || c == '.' {
            number.push(c);
            continue;
        }
        let unit = match c {
            'h' => 3600.0,
            'm' => 60.0,
            's' => 1.0,
            ' ' => continue,
            _ => {
                return Err(format!(
                    "{text:?} isn't a duration (say 90, \"45s\" or \"1m30s\")"
                ));
            }
        };
        let n: f64 = number
            .parse()
            .map_err(|_| format!("{text:?} isn't a duration (say 90, \"45s\" or \"1m30s\")"))?;
        total += n * unit;
        number.clear();
    }
    if !number.is_empty() {
        return Err(format!("{text:?} has a number without a unit at its end"));
    }
    Ok(total)
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStep {
    think: Option<String>,
    say: Option<String>,
    bash: Option<String>,
    run: Option<String>,
    read: Option<String>,
    edit: Option<String>,
    write: Option<String>,
    grep: Option<String>,
    glob: Option<String>,
    tool: Option<String>,
    tools: Option<Vec<RawStep>>,
    todo: Option<Vec<Vec<String>>>,
    agent: Option<String>,
    commit: Option<String>,
    push: Option<bool>,
    pr: Option<String>,
    limit: Option<Seconds>,
    usage: Option<RawUsage>,
    pause: Option<Seconds>,
    silently: Option<String>,
    context: Option<u64>,
    // The details.
    description: Option<String>,
    output: Option<String>,
    error: Option<bool>,
    seconds: Option<Seconds>,
    hold: Option<bool>,
    show: Option<String>,
    old: Option<String>,
    new: Option<String>,
    replace_all: Option<bool>,
    content: Option<String>,
    path: Option<String>,
    mode: Option<String>,
    input: Option<toml::Value>,
    prompt: Option<String>,
    report: Option<String>,
    steps: Option<Vec<RawStep>>,
    number: Option<u64>,
    body: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUsage {
    five_hour: f64,
    seven_day: f64,
}

/// The keys that say what a step is.
const KINDS: [&str; 21] = [
    "think", "say", "bash", "run", "read", "edit", "write", "grep", "glob", "tool", "tools",
    "todo", "agent", "commit", "push", "pr", "limit", "usage", "pause", "silently", "context",
];

impl RawStep {
    fn kinds(&self) -> Vec<&'static str> {
        let set = [
            self.think.is_some(),
            self.say.is_some(),
            self.bash.is_some(),
            self.run.is_some(),
            self.read.is_some(),
            self.edit.is_some(),
            self.write.is_some(),
            self.grep.is_some(),
            self.glob.is_some(),
            self.tool.is_some(),
            self.tools.is_some(),
            self.todo.is_some(),
            self.agent.is_some(),
            self.commit.is_some(),
            self.push.is_some(),
            self.pr.is_some(),
            self.limit.is_some(),
            self.usage.is_some(),
            self.pause.is_some(),
            self.silently.is_some(),
            self.context.is_some(),
        ];
        KINDS
            .iter()
            .zip(set)
            .filter(|(_, on)| *on)
            .map(|(k, _)| *k)
            .collect()
    }
}

fn step(s: RawStep) -> Result<Step, String> {
    let kinds = s.kinds();
    let [kind] = kinds.as_slice() else {
        return Err(if kinds.is_empty() {
            format!("it says none of {}", KINDS.join(", "))
        } else {
            format!("it says {} at once: one per step", kinds.join(" and "))
        });
    };
    Ok(match *kind {
        "think" => Step::Think(s.think.unwrap_or_default()),
        "say" => Step::Say(s.say.unwrap_or_default()),
        "tools" => {
            let calls = s
                .tools
                .unwrap_or_default()
                .into_iter()
                .map(call)
                .collect::<Result<Vec<_>, _>>()?;
            if calls.is_empty() {
                return Err("`tools` is empty".into());
            }
            Step::Calls(calls)
        }
        "agent" => {
            let mut steps = Vec::new();
            for sub in s.steps.unwrap_or_default() {
                steps.push(match sub.say {
                    Some(text) if sub.kinds().len() == 1 => AgentStep::Say(text),
                    _ => AgentStep::Call(call(sub)?),
                });
            }
            Step::Agent(Agent {
                description: s.agent.unwrap_or_default(),
                prompt: s.prompt.unwrap_or_default(),
                steps,
                report: s.report.unwrap_or_default(),
            })
        }
        "limit" => Step::Limit(s.limit.map(|d| d.parse()).transpose()?.unwrap_or_default()),
        "usage" => {
            let u = s.usage.ok_or("`usage` is empty")?;
            Step::Usage {
                five_hour: u.five_hour,
                seven_day: u.seven_day,
            }
        }
        "pause" => Step::Pause(s.pause.map(|d| d.parse()).transpose()?.unwrap_or_default()),
        "silently" => Step::Silently(s.silently.unwrap_or_default()),
        "context" => Step::Context(s.context.unwrap_or_default()),
        _ => Step::Calls(vec![call(s)?]),
    })
}

/// A step that is a tool call.
fn call(s: RawStep) -> Result<Call, String> {
    let kinds = s.kinds();
    let kind = match kinds.as_slice() {
        [kind] => *kind,
        _ => {
            return Err(format!(
                "a call says {}: one of bash, run, read, edit, write, grep, glob, tool, todo, commit, push, pr",
                kinds.join(" and ")
            ));
        }
    };
    let description = s.description.clone();
    let described = |command: &str| {
        let mut input = json!({"command": command});
        if let Some(d) = &description {
            input["description"] = json!(d);
        }
        input
    };
    let (name, input, effect) = match kind {
        "bash" => {
            let command = s.bash.clone().unwrap_or_default();
            ("Bash".to_owned(), described(&command), Effect::None)
        }
        "run" => {
            let command = s.run.clone().unwrap_or_default();
            let shown = s.show.clone().unwrap_or_else(|| command.clone());
            ("Bash".to_owned(), described(&shown), Effect::Run(command))
        }
        "read" => {
            let path = s.read.clone().unwrap_or_default();
            (
                "Read".to_owned(),
                json!({"file_path": path}),
                Effect::Read(path),
            )
        }
        "edit" => {
            let path = s.edit.clone().unwrap_or_default();
            let (Some(old), Some(new)) = (s.old.clone(), s.new.clone()) else {
                return Err(format!("the edit of {path} needs `old` and `new`"));
            };
            let all = s.replace_all.unwrap_or(false);
            let mut input = json!({"file_path": path, "old_string": old, "new_string": new});
            if all {
                input["replace_all"] = json!(true);
            }
            (
                "Edit".to_owned(),
                input,
                Effect::Edit {
                    path,
                    old,
                    new,
                    all,
                },
            )
        }
        "write" => {
            let path = s.write.clone().unwrap_or_default();
            let content = s.content.clone().unwrap_or_default();
            (
                "Write".to_owned(),
                json!({"file_path": path, "content": content}),
                Effect::Write { path, content },
            )
        }
        "grep" => {
            let mut input = json!({"pattern": s.grep.clone().unwrap_or_default(), "output_mode": s.mode.clone().unwrap_or_else(|| "content".into()), "-n": true});
            if let Some(p) = &s.path {
                input["path"] = json!(p);
            }
            ("Grep".to_owned(), input, Effect::None)
        }
        "glob" => {
            let mut input = json!({"pattern": s.glob.clone().unwrap_or_default()});
            if let Some(p) = &s.path {
                input["path"] = json!(p);
            }
            ("Glob".to_owned(), input, Effect::None)
        }
        "tool" => (
            s.tool.clone().unwrap_or_default(),
            s.input
                .clone()
                .map(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
                .transpose()?
                .unwrap_or_else(|| json!({})),
            Effect::None,
        ),
        "todo" => {
            let mut todos = Vec::new();
            for pair in s.todo.clone().unwrap_or_default() {
                let [content, status] = pair.as_slice() else {
                    return Err("a todo is `[content, status]`".into());
                };
                if !matches!(status.as_str(), "pending" | "in_progress" | "completed") {
                    return Err(format!(
                        "a todo's status is pending, in_progress or completed, not {status:?}"
                    ));
                }
                todos.push(json!({"content": content, "status": status, "activeForm": content}));
            }
            (
                "TodoWrite".to_owned(),
                json!({"todos": todos}),
                Effect::None,
            )
        }
        "commit" => {
            let message = s.commit.clone().unwrap_or_default();
            ("Bash".to_owned(), json!({}), Effect::Commit(message))
        }
        "push" => (
            "Bash".to_owned(),
            json!({"command": "git push -u origin HEAD", "description": "Push the branch"}),
            Effect::Push,
        ),
        "pr" => {
            let title = s.pr.clone().unwrap_or_default();
            let number = s.number.ok_or("a `pr` needs its `number`")?;
            let body = s.body.clone().unwrap_or_default();
            let command = format!(
                "gh pr create --title {title:?} --body \"$(cat <<'EOF'\n{}\nEOF\n)\"",
                body.trim_end()
            );
            (
                "Bash".to_owned(),
                json!({"command": command, "description": "Open the pull request"}),
                Effect::PullRequest { number },
            )
        }
        other => return Err(format!("{other} isn't a tool call")),
    };
    let output = match kind {
        "todo" => Some(s.output.clone().unwrap_or_else(|| "Todos have been modified successfully. Ensure that you continue to use the todo list to track your progress. Please proceed with the current tasks if applicable".into())),
        _ => s.output.clone(),
    };
    Ok(Call {
        name,
        input,
        effect,
        output,
        error: s.error.unwrap_or(false),
        seconds: s.seconds.map(|d| d.parse()).transpose()?,
        hold: s.hold.unwrap_or(false),
    })
}

/// `cat -n` of `text`, as Claude Code's `Read` returns a file.
pub fn numbered(text: &str) -> String {
    text.lines()
        .zip(1..)
        .map(|(line, n)| format!("{n:>6}\t{line}\n"))
        .collect()
}

/// The GitHub repository `origin` names (`owner/name`), from its URL.
pub fn github_repo(origin: &str) -> Option<String> {
    let rest = origin
        .trim()
        .strip_prefix("https://github.com/")
        .or_else(|| origin.trim().strip_prefix("git@github.com:"))?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let (owner, name) = rest.split_once('/')?;
    (!owner.is_empty() && !name.is_empty() && !name.contains('/'))
        .then(|| format!("{owner}/{name}"))
}

/// What `sh -c command` printed in the working directory, with `env`
/// added, and whether it failed.
fn sh_with(command: &str, env: &[(String, String)]) -> (String, bool) {
    match std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .output()
    {
        Ok(o) => (
            String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr),
            !o.status.success(),
        ),
        Err(e) => (format!("sh: {e}"), true),
    }
}

/// `path` in the working directory, as Claude Code names files: absolute.
fn absolute(path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_owned()
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    }
}

/// A pause of `base` at `speed`.
fn paced(base: Duration, speed: f64) -> Duration {
    base.div_f64(speed)
}

/// What fake-claude's commit messages end with, as Claude Code's do.
fn commit_message(message: &str) -> String {
    match crate::commit_attribution() {
        Some(trailers) => format!("{}\n\n{trailers}", message.trim_end()),
        None => message.trim_end().to_owned(),
    }
}

impl crate::Session {
    /// The turn of a replay that answers `text`: the next of the one under
    /// way, else the first of the one `text` starts.
    pub(crate) fn replay_turn(&mut self, text: &str) -> Option<Turn> {
        if let Some(turn) = self.playing.as_mut().and_then(|p| p.take(text)) {
            return Some(turn);
        }
        let mut playing = self.library.as_ref()?.start(text)?;
        let turn = playing.take(text)?;
        self.playing = Some(playing);
        Some(turn)
    }

    /// Plays a turn, then its result.
    pub(crate) fn play(&mut self, turn: Turn, started: std::time::Instant) {
        let (name, number) = self
            .playing
            .as_ref()
            .map_or((String::new(), 0), |p| (p.replay.name.clone(), p.next));
        let (speed, env) = self
            .library
            .as_ref()
            .map_or((1.0, Vec::new()), |l| (l.speed(), l.env()));
        let delay = self.delay;
        self.delay = paced(delay, speed);
        let mut said = None;
        let mut interrupted = false;
        for step in &turn.steps {
            if crate::INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst) {
                interrupted = true;
                break;
            }
            match step {
                Step::Think(t) => self.think(t),
                Step::Say(t) => {
                    self.stream_text(t);
                    said = Some(t.clone());
                }
                Step::Calls(calls) => {
                    if calls.iter().any(|c| c.hold) {
                        note(&format!("{name}: holds"));
                    }
                    if !self.play_calls(calls, speed, &env) {
                        interrupted = true;
                        break;
                    }
                }
                Step::Agent(a) => {
                    if !self.play_agent(a, speed, &env) {
                        interrupted = true;
                        break;
                    }
                }
                Step::Limit(resets_in) => {
                    self.delay = delay;
                    self.hit_limit(*resets_in, started);
                    note(&format!("{name}: limited"));
                    return;
                }
                Step::Usage {
                    five_hour,
                    seven_day,
                } => self.report_usage(*five_hour, *seven_day),
                Step::Pause(d) => std::thread::sleep(paced(*d, speed)),
                Step::Silently(command) => {
                    let (output, failed) = sh_with(command, &env);
                    if failed {
                        warn(&format!("{command:?} failed: {}", output.trim()));
                    }
                }
                Step::Context(tokens) => {
                    self.context = (*tokens).clamp(crate::CONTEXT_START, crate::CONTEXT_WINDOW);
                }
            }
            // A beat between steps, as each of Claude's calls takes one.
            std::thread::sleep(paced(Duration::from_millis(300), speed));
        }
        self.delay = delay;
        if interrupted {
            self.result(started, "Interrupted", "error_during_execution");
            return;
        }
        let text = turn.result.or(said).unwrap_or_else(|| "Done.".to_owned());
        self.result_took(started, &text, "success", turn.duration);
        note(&format!("{name}: turn {number} played"));
    }

    /// The input a call shows: files by their absolute paths, and commits
    /// with the message Claude Code writes.
    fn shown_input(c: &Call) -> Value {
        let mut input = c.input.clone();
        if let Some(path) = input.get("file_path").and_then(Value::as_str) {
            input["file_path"] = json!(absolute(path).display().to_string());
        }
        if let Effect::Commit(message) = &c.effect {
            input = json!({
                "command": format!("git add -A && git commit -m \"$(cat <<'EOF'\n{}\nEOF\n)\"", commit_message(message)),
                "description": "Commit the changes",
            });
        }
        input
    }

    /// What a call does for real, and what it prints.
    fn perform(c: &Call, env: &[(String, String)]) -> (String, bool) {
        let (output, error) = match &c.effect {
            Effect::None => (String::new(), false),
            Effect::Run(command) => sh_with(command, env),
            Effect::Read(path) => match std::fs::read_to_string(absolute(path)) {
                Ok(text) => (numbered(&text), false),
                Err(_) if c.output.is_some() => (String::new(), false),
                Err(_) => (
                    "<tool_use_error>File does not exist.</tool_use_error>".to_owned(),
                    true,
                ),
            },
            Effect::Edit {
                path,
                old,
                new,
                all,
            } => {
                let file = absolute(path);
                match std::fs::read_to_string(&file) {
                    Ok(text) if text.contains(old.as_str()) => {
                        let changed = if *all {
                            text.replace(old.as_str(), new)
                        } else {
                            text.replacen(old.as_str(), new, 1)
                        };
                        if let Err(e) = std::fs::write(&file, changed) {
                            warn(&format!("editing {}: {e}", file.display()));
                        }
                    }
                    _ => warn(&format!(
                        "{} doesn't have what the replay's edit replaces: only the transcript shows it",
                        file.display()
                    )),
                }
                (
                    format!("The file {} has been updated successfully.", file.display()),
                    false,
                )
            }
            Effect::Write { path, content } => {
                let file = absolute(path);
                let existed = file.exists();
                let written = file
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&file, content));
                match written {
                    Ok(()) if existed => (
                        format!("The file {} has been updated successfully.", file.display()),
                        false,
                    ),
                    Ok(()) => (
                        format!("File created successfully at: {}", file.display()),
                        false,
                    ),
                    Err(e) => (format!("Error writing {}: {e}", file.display()), true),
                }
            }
            Effect::Commit(message) => {
                let mut env = env.to_vec();
                env.push(("FAKE_CLAUDE_MESSAGE".to_owned(), commit_message(message)));
                sh_with("git add -A && git commit -m \"$FAKE_CLAUDE_MESSAGE\"", &env)
            }
            Effect::Push => {
                let (output, failed) = sh_with("git push -u origin HEAD", env);
                if failed {
                    // As it would have printed, from GitHub.
                    warn(&format!("git push failed: {}", output.trim()));
                    let (branch, _) = sh_with("git rev-parse --abbrev-ref HEAD", env);
                    let (origin, _) = sh_with("git remote get-url origin", env);
                    let branch = branch.trim();
                    let repo = github_repo(&origin).unwrap_or_else(|| "owner/repository".into());
                    (
                        format!(
                            "To https://github.com/{repo}.git\n * [new branch]      {branch} -> {branch}\nbranch '{branch}' set up to track 'origin/{branch}'.\n"
                        ),
                        false,
                    )
                } else {
                    (output, false)
                }
            }
            Effect::PullRequest { number } => {
                let (origin, _) = sh_with("git remote get-url origin", env);
                let repo = github_repo(&origin).unwrap_or_else(|| "owner/repository".into());
                (format!("https://github.com/{repo}/pull/{number}\n"), false)
            }
        };
        match &c.output {
            Some(written) => (written.clone(), c.error),
            None => (output, error || c.error),
        }
    }

    /// Calls tools as one response asks for them: each call, then their
    /// results in order. Returns false when it was interrupted, or a call
    /// holds until it is.
    fn play_calls(&mut self, calls: &[Call], speed: f64, env: &[(String, String)]) -> bool {
        let msg_id = format!("msg_{}", crate::short_id(8));
        let usage = self.call_started();
        let ids: Vec<String> = calls
            .iter()
            .map(|_| format!("toolu_{}", crate::short_id(12)))
            .collect();
        for (c, id) in calls.iter().zip(&ids) {
            crate::out(
                &json!({"type": "assistant", "message": {"id": msg_id, "type": "message", "role": "assistant", "model": self.model, "content": [{"type": "tool_use", "id": id, "name": c.name, "input": Self::shown_input(c)}], "stop_reason": "tool_use", "usage": usage}, "parent_tool_use_id": null, "session_id": self.id, "uuid": crate::uuid()}),
            );
        }
        self.call_ended(&msg_id, "tool_use", 40 * calls.len() as u64);
        for (c, id) in calls.iter().zip(&ids) {
            if c.hold {
                self.hold(id, &c.name);
                crate::out(
                    &json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "content": "Interrupted", "is_error": true}]}, "parent_tool_use_id": null, "session_id": self.id, "uuid": crate::uuid()}),
                );
                return false;
            }
            let (output, error) = Self::perform(c, env);
            self.tick(id, &c.name, c.seconds.map(|d| paced(d, speed)), None);
            crate::out(
                &json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "content": output, "is_error": error}]}, "parent_tool_use_id": null, "session_id": self.id, "uuid": crate::uuid()}),
            );
        }
        !crate::INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// A call's progress, as Claude Code reports it while a tool runs: each
    /// second of `length`, or a couple of beats.
    fn tick(&mut self, id: &str, name: &str, length: Option<Duration>, parent: Option<&str>) {
        let Some(length) = length else {
            for i in 0..2 {
                crate::out(
                    &json!({"type": "tool_progress", "tool_use_id": id, "tool_name": name, "parent_tool_use_id": parent, "elapsed_time_seconds": i, "session_id": self.id, "uuid": crate::uuid()}),
                );
                self.sleep();
            }
            return;
        };
        let started = std::time::Instant::now();
        let mut reported = 0;
        while started.elapsed() < length {
            if crate::INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            if started.elapsed().as_secs() >= reported {
                crate::out(
                    &json!({"type": "tool_progress", "tool_use_id": id, "tool_name": name, "parent_tool_use_id": parent, "elapsed_time_seconds": started.elapsed().as_secs(), "session_id": self.id, "uuid": crate::uuid()}),
                );
                reported = started.elapsed().as_secs() + 1;
            }
            std::thread::sleep((length - started.elapsed()).min(Duration::from_millis(200)));
        }
    }

    /// A call that runs until someone interrupts it.
    fn hold(&mut self, id: &str, name: &str) {
        let started = std::time::Instant::now();
        let mut reported = 0;
        while !crate::INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst) {
            if started.elapsed().as_secs() >= reported {
                crate::out(
                    &json!({"type": "tool_progress", "tool_use_id": id, "tool_name": name, "parent_tool_use_id": null, "elapsed_time_seconds": started.elapsed().as_secs(), "session_id": self.id, "uuid": crate::uuid()}),
                );
                reported = started.elapsed().as_secs() + 1;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// A subagent that does `a`'s steps, which the `Agent` call waits for.
    /// Returns false when it was interrupted.
    fn play_agent(&mut self, a: &Agent, speed: f64, env: &[(String, String)]) -> bool {
        let mut agent = self.start_subagent_as(false, &a.description, &a.prompt);
        for step in &a.steps {
            if crate::INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            match step {
                AgentStep::Say(text) => {
                    self.subagent_says(&agent, json!([{"type": "text", "text": text}]));
                }
                AgentStep::Call(c) => {
                    agent.tool_uses += 1;
                    let what = c.input["description"]
                        .as_str()
                        .or_else(|| c.input["command"].as_str())
                        .unwrap_or(&c.name)
                        .to_owned();
                    crate::out(
                        &json!({"type": "system", "subtype": "task_progress", "task_id": agent.task_id, "tool_use_id": agent.tool_use_id, "description": format!("Running {what}"), "last_tool_name": c.name, "usage": agent.usage(), "session_id": self.id, "uuid": crate::uuid()}),
                    );
                    let tool_id = format!("toolu_{}", crate::short_id(12));
                    self.subagent_says(
                        &agent,
                        json!([{"type": "tool_use", "id": tool_id, "name": c.name, "input": Self::shown_input(c)}]),
                    );
                    let (output, error) = Self::perform(c, env);
                    let parent = agent.tool_use_id.clone();
                    self.tick(
                        &tool_id,
                        &c.name,
                        c.seconds.map(|d| paced(d, speed)),
                        Some(&parent),
                    );
                    crate::out(
                        &json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": tool_id, "content": output, "is_error": error}]}, "parent_tool_use_id": agent.tool_use_id, "session_id": self.id, "uuid": crate::uuid()}),
                    );
                }
            }
            std::thread::sleep(paced(Duration::from_millis(500), speed));
        }
        let interrupted = crate::INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst);
        self.end_subagent_with(
            &agent,
            if interrupted { "stopped" } else { "completed" },
            false,
            &a.report,
        );
        let mut result = if interrupted {
            json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": agent.tool_use_id, "content": "[Request interrupted by user for tool use]", "is_error": true}]}})
        } else {
            let usage = agent.usage();
            let text = format!(
                "{}\nagentId: {} (use SendMessage with to: '{}' to continue this agent)\n<usage>tool_uses: {}\nduration_ms: {}</usage>",
                a.report, agent.task_id, agent.task_id, usage["tool_uses"], usage["duration_ms"]
            );
            json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": agent.tool_use_id, "content": [{"type": "text", "text": text}], "is_error": false}]},
                "tool_use_result": {"status": "completed", "agentId": agent.task_id, "agentType": "general-purpose", "content": [{"type": "text", "text": a.report}], "totalDurationMs": usage["duration_ms"], "totalToolUseCount": agent.tool_uses}})
        };
        result["parent_tool_use_id"] = Value::Null;
        result["session_id"] = json!(self.id);
        result["uuid"] = json!(crate::uuid());
        crate::out(&result);
        !interrupted
    }

    /// The five-hour limit, which resets `resets_in` from now: the turn
    /// ends there, as Claude Code's does.
    fn hit_limit(&mut self, resets_in: Duration, started: std::time::Instant) {
        let resets = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d + resets_in)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        crate::out(
            &json!({"type": "rate_limit_event", "rate_limit_info": {"status": "rejected", "resetsAt": resets, "rateLimitType": "five_hour", "utilization": 1.0}, "uuid": crate::uuid(), "session_id": self.id}),
        );
        crate::out(
            &json!({"type": "assistant", "message": {"id": "msg_limit", "type": "message", "role": "assistant", "model": self.model, "content": [{"type": "text", "text": "Claude usage limit reached."}], "usage": {"input_tokens": 0, "output_tokens": 0}}, "error": "rate_limit", "parent_tool_use_id": null, "session_id": self.id, "uuid": crate::uuid()}),
        );
        self.result(
            started,
            "Claude usage limit reached.",
            "error_during_execution",
        );
    }

    /// Plan usage as given, as Claude Code reports it after a response: each
    /// window resets where the clock's would ([`crate::Session::usage`]).
    fn report_usage(&mut self, five_hour: f64, seven_day: f64) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let resets = |secs: u64| now - now % secs + secs;
        crate::out(
            &json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "isUsingOverage": false, "unifiedWindows": {"five_hour": {"utilization": five_hour, "resetsAt": resets(5 * 3600)}, "seven_day": {"utilization": seven_day, "resetsAt": resets(7 * 86_400)}}}, "uuid": crate::uuid(), "session_id": self.id}),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{Effect, Library, Step, github_repo, numbered, parse, parse_duration, parse_env};
    use std::time::Duration;

    #[test]
    fn durations_read_as_people_write_them() {
        assert_eq!(parse_duration("90"), Ok(90.0));
        assert_eq!(parse_duration("45s"), Ok(45.0));
        assert_eq!(parse_duration("1m30s"), Ok(90.0));
        assert_eq!(parse_duration("2h"), Ok(7200.0));
        assert_eq!(parse_duration("1h 5m"), Ok(3900.0));
        assert!(parse_duration("1m30").is_err());
        assert!(parse_duration("soon").is_err());
    }

    #[test]
    fn a_replay_reads_its_turns_and_steps() -> Result<(), String> {
        let r = parse(
            r#"
match = "Add Danish"

[[turn]]
duration = "14m"
[[turn.step]]
think = "Prices first."
[[turn.step]]
bash = "npm test"
description = "Run the tests"
output = "186 passed"
[[turn.step]]
tools = [
  { read = "lib/money.ts" },
  { grep = "formatPrice", path = "src", output = "src/a.ts:1" },
]
[[turn.step]]
edit = "lib/money.ts"
old = "SEK"
new = "SEK | DKK"
[[turn.step]]
todo = [["Read the ticket", "completed"], ["Add da", "in_progress"]]
[[turn.step]]
agent = "Check the prices"
prompt = "Find every price."
report = "Three places."
  [[turn.step.steps]]
  say = "Looking."
  [[turn.step.steps]]
  bash = "grep -rn price"
  output = "a.ts:3"
[[turn.step]]
bash = "gh pr checks 1291 --watch"
hold = true

[[turn]]
match = "page someone"
[[turn.step]]
say = "The daily summary it is."
"#,
        )?;
        assert_eq!(r.matches, "Add Danish");
        let [first, second] = r.turns.as_slice() else {
            return Err(format!("{} turns", r.turns.len()));
        };
        assert_eq!(first.duration, Some(Duration::from_secs(14 * 60)));
        assert_eq!(first.steps.len(), 7);
        assert!(
            matches!(&first.steps[1], Step::Calls(c) if c[0].name == "Bash" && c[0].input["description"] == "Run the tests")
        );
        assert!(matches!(&first.steps[2], Step::Calls(c) if c.len() == 2 && c[1].name == "Grep"));
        assert!(
            matches!(&first.steps[3], Step::Calls(c) if matches!(&c[0].effect, Effect::Edit { new, .. } if new == "SEK | DKK"))
        );
        assert!(
            matches!(&first.steps[4], Step::Calls(c) if c[0].name == "TodoWrite" && c[0].input["todos"][1]["status"] == "in_progress")
        );
        assert!(
            matches!(&first.steps[5], Step::Agent(a) if a.steps.len() == 2 && a.report == "Three places.")
        );
        assert!(matches!(&first.steps[6], Step::Calls(c) if c[0].hold));
        assert_eq!(second.matches.as_deref(), Some("page someone"));
        Ok(())
    }

    #[test]
    fn a_step_says_one_thing() {
        let both = "match = \"x\"\n[[turn]]\n[[turn.step]]\nsay = \"a\"\nthink = \"b\"\n";
        assert!(parse(both).is_err_and(|e| e.contains("think and say")
            || e.contains("say and think")
            || e.contains("at once")));
        let none = "match = \"x\"\n[[turn]]\n[[turn.step]]\ndescription = \"a\"\n";
        assert!(parse(none).is_err_and(|e| e.contains("none of")));
        let unknown = "match = \"x\"\n[[turn]]\n[[turn.step]]\nshout = \"a\"\n";
        assert!(parse(unknown).is_err());
        assert!(parse("match = \"x\"\n").is_err_and(|e| e.contains("[[turn]]")));
        let edit = "match = \"x\"\n[[turn]]\n[[turn.step]]\nedit = \"a.rs\"\nold = \"a\"\n";
        assert!(parse(edit).is_err_and(|e| e.contains("`old` and `new`")));
    }

    #[test]
    fn turns_answer_the_messages_they_match() -> Result<(), String> {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let write = |name: &str, text: &str| {
            std::fs::write(dir.path().join(name), text).map_err(|e| e.to_string())
        };
        write(
            "shop.toml",
            "match = \"Add Danish to the shop\"\n[[turn]]\n[[turn.step]]\nsay = \"On it.\"\n[[turn]]\n[[turn.step]]\nsay = \"Next.\"\n[[turn]]\nmatch = \"review agent reviewed\"\n[[turn.step]]\nsay = \"Fixed.\"\n",
        )?;
        write("broken.toml", "match = \n")?;
        write("speed", "20\n")?;
        write("env", "# for the commands\nGIT_CONFIG_COUNT=1\n\nA=b=c\n")?;
        let lib = Library::load(dir.path());
        assert_eq!(lib.replays.len(), 1);
        assert!((lib.speed() - 20.0).abs() < f64::EPSILON);
        assert_eq!(
            lib.env(),
            [
                ("GIT_CONFIG_COUNT".into(), "1".into()),
                ("A".into(), "b=c".into())
            ]
        );
        assert!(lib.start("write notes.md").is_none());
        let mut playing = lib
            .start("Tomás delegated SHOP-419: ADD DANISH TO THE SHOP")
            .ok_or("no replay started")?;
        let first = playing
            .take("Tomás delegated SHOP-419: ADD DANISH TO THE SHOP")
            .ok_or("no first turn")?;
        assert!(matches!(&first.steps[0], Step::Say(s) if s == "On it."));
        // The next turn takes any message.
        assert!(playing.take("go on").is_some());
        // The last one only its own.
        assert!(playing.take("thanks").is_none());
        assert!(
            playing
                .take("fctl's review agent reviewed commit 3f2a1c9")
                .is_some()
        );
        assert!(playing.take("anything").is_none());
        Ok(())
    }

    #[test]
    fn replays_with_one_match_play_once_each() -> Result<(), String> {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        for (name, says) in [("review-1", "1 issue."), ("review-2", "No issues.")] {
            std::fs::write(
                dir.path().join(format!("{name}.toml")),
                format!(
                    "match = \"Review acme/web#7\"\n[[turn]]\n[[turn.step]]\nsay = \"{says}\"\n"
                ),
            )
            .map_err(|e| e.to_string())?;
        }
        let lib = Library::load(dir.path());
        let name = |text: &str| lib.start(text).map(|p| p.replay.name);
        assert_eq!(
            name("Review acme/web#7 at commit 1a2b3c4."),
            Some("review-1".into())
        );
        assert_eq!(
            name("Review acme/web#7 at commit 5d6e7f8."),
            Some("review-2".into())
        );
        // Once all have started, the first again.
        assert_eq!(
            name("Review acme/web#7 at commit 9a0b1c2."),
            Some("review-1".into())
        );
        Ok(())
    }

    /// Every transcript `cargo xtask demo` plays parses.
    #[test]
    fn the_demo_transcripts_parse() -> Result<(), String> {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../xtask/demo/transcripts");
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        files.sort();
        assert!(!files.is_empty(), "no transcripts in {}", dir.display());
        let mut wrong = Vec::new();
        for path in files {
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            match parse(&text) {
                Ok(r) if r.turns.is_empty() => wrong.push(format!("{}: no turns", path.display())),
                Ok(_) => {}
                Err(e) => wrong.push(format!("{}: {e}", path.display())),
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
        Ok(())
    }

    #[test]
    fn later_turns_skip_to_the_one_a_message_matches() -> Result<(), String> {
        let r = parse(
            "match = \"start\"\n[[turn]]\n[[turn.step]]\nsay = \"1\"\n[[turn]]\nmatch = \"second\"\n[[turn.step]]\nsay = \"2\"\n[[turn]]\nmatch = \"third\"\n[[turn.step]]\nsay = \"3\"\n",
        )?;
        let mut playing = super::Playing { replay: r, next: 1 };
        let turn = playing.take("the third one").ok_or("no turn")?;
        assert!(matches!(&turn.steps[0], Step::Say(s) if s == "3"));
        assert!(playing.take("the second one").is_none());
        Ok(())
    }

    #[test]
    fn reads_number_lines_and_origins_name_repositories() {
        assert_eq!(numbered("a\nb\n"), "     1\ta\n     2\tb\n");
        assert_eq!(
            github_repo("https://github.com/romanescohq/shop.git").as_deref(),
            Some("romanescohq/shop")
        );
        assert_eq!(
            github_repo("git@github.com:milkfloat/milkfloat.dev.git").as_deref(),
            Some("milkfloat/milkfloat.dev")
        );
        assert_eq!(github_repo("/tmp/shop.git"), None);
        assert_eq!(parse_env("X=1"), [("X".to_owned(), "1".to_owned())]);
    }
}
