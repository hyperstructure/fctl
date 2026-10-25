//! fake-claude driven as fctl-agent drives Claude Code: user turns on its
//! stdin, messages on its stdout.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn results_count_the_turns_still_waiting() -> Result<(), Box<dyn Error>> {
    let mut claude = Command::new(env!("CARGO_BIN_EXE_fake-claude"))
        .env("FAKE_CLAUDE_DELAY_MS", "10")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let (Some(mut stdin), Some(stdout)) = (claude.stdin.take(), claude.stdout.take()) else {
        return Err("fake-claude's stdin and stdout aren't piped".into());
    };
    // All three at once, as fctl-agent sends its message after a usage limit
    // resets and the ones that waited for it: two arrive during the first turn.
    let turns: String = ["one", "two", "three"]
        .iter()
        .map(|text| {
            format!(
                "{}\n",
                json!({"type": "user", "message": {"role": "user", "content": text}})
            )
        })
        .collect();
    stdin.write_all(turns.as_bytes())?;
    let mut waiting = Vec::new();
    for line in BufReader::new(stdout).lines() {
        let v: Value = serde_json::from_str(&line?)?;
        if v["type"] == "result" {
            waiting.push(v["queued_turn_count"].clone());
            if waiting.len() == 3 {
                break;
            }
        }
    }
    drop(stdin);
    claude.wait()?;
    assert_eq!(waiting, [json!(2), json!(1), json!(0)]);
    Ok(())
}

#[test]
fn warns_about_the_week_as_claude_code_does() -> Result<(), Box<dyn Error>> {
    let mut claude = Command::new(env!("CARGO_BIN_EXE_fake-claude"))
        .env("FAKE_CLAUDE_DELAY_MS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let (Some(mut stdin), Some(stdout)) = (claude.stdin.take(), claude.stdout.take()) else {
        return Err("fake-claude's stdin and stdout aren't piped".into());
    };
    let turns: String = ["Hello", "A warning, please"]
        .iter()
        .map(|text| {
            format!(
                "{}\n",
                json!({"type": "user", "message": {"role": "user", "content": text}})
            )
        })
        .collect();
    stdin.write_all(turns.as_bytes())?;
    let mut reports = Vec::new();
    let mut results = 0;
    for line in BufReader::new(stdout).lines() {
        let v: Value = serde_json::from_str(&line?)?;
        match v["type"].as_str() {
            Some("rate_limit_event") => reports.push(v["rate_limit_info"].clone()),
            Some("result") => results += 1,
            _ => {}
        }
        if results == 2 {
            break;
        }
    }
    drop(stdin);
    claude.wait()?;
    let [plain, warned] = reports.as_slice() else {
        return Err(format!("{} usage reports", reports.len()).into());
    };
    assert_eq!(plain["status"], "allowed");
    assert_eq!(plain.get("rateLimitType"), None);
    // The week's numbers come beside the status too.
    assert_eq!(warned["status"], "allowed_warning");
    assert_eq!(warned["rateLimitType"], "seven_day");
    let week = &warned["unifiedWindows"]["seven_day"];
    assert!(week["utilization"].is_f64());
    assert_eq!(warned["utilization"], week["utilization"]);
    assert_eq!(warned["resetsAt"], week["resetsAt"]);
    Ok(())
}

/// What fake-claude says of its state, its turns and its one task after
/// `turn`, as fctl-agent runs it: the first `n` of them.
fn states_and_turns(turn: &str, n: usize) -> Result<Vec<String>, Box<dyn Error>> {
    let mut claude = Command::new(env!("CARGO_BIN_EXE_fake-claude"))
        .env("FAKE_CLAUDE_DELAY_MS", "1")
        .env("CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let (Some(mut stdin), Some(stdout)) = (claude.stdin.take(), claude.stdout.take()) else {
        return Err("fake-claude's stdin and stdout aren't piped".into());
    };
    let turn = json!({"type": "user", "message": {"role": "user", "content": turn}});
    stdin.write_all(format!("{turn}\n").as_bytes())?;
    let mut seen = Vec::new();
    let mut task = None;
    for line in BufReader::new(stdout).lines() {
        let v: Value = serde_json::from_str(&line?)?;
        let what = match (v["type"].as_str(), v["subtype"].as_str()) {
            (Some("system"), Some("session_state_changed")) => {
                v["state"].as_str().map(str::to_owned)
            }
            (Some("system"), Some("init")) => Some("init".to_owned()),
            (Some("system"), Some(s @ ("task_started" | "task_notification"))) => {
                let id = v["task_id"].clone();
                assert!(
                    task.get_or_insert_with(|| id.clone()) == &id,
                    "one task: {v}"
                );
                Some(s.to_owned())
            }
            (Some("result"), _) => Some("result".to_owned()),
            _ => None,
        };
        seen.extend(what);
        if seen.len() == n {
            break;
        }
    }
    drop(stdin);
    claude.wait()?;
    Ok(seen)
}

#[test]
fn background_work_ends_in_a_turn_of_its_own() -> Result<(), Box<dyn Error>> {
    // The turn, the command's end, and the turn that takes its result.
    assert_eq!(
        states_and_turns("Run it in the background 1", 10)?,
        [
            "running",
            "init",
            "task_started",
            "result",
            "idle",
            "task_notification",
            "running",
            "init",
            "result",
            "idle"
        ]
    );
    Ok(())
}

#[test]
fn a_subagent_in_the_background_keeps_claude_running() -> Result<(), Box<dyn Error>> {
    // As Claude Code 2.1.308 says: `running` from the turn that starts the
    // subagent until the turn that takes its report has ended, which
    // starts with `system/init` alone.
    assert_eq!(
        states_and_turns("Delegate it in the background", 8)?,
        [
            "running",
            "init",
            "task_started",
            "result",
            "task_notification",
            "init",
            "result",
            "idle"
        ]
    );
    Ok(())
}

/// What fake-claude sends for a turn, without its stream events, until
/// `until_results` turns have ended.
fn subagent_messages(turn: &str, until_results: usize) -> Result<Vec<Value>, Box<dyn Error>> {
    let mut claude = Command::new(env!("CARGO_BIN_EXE_fake-claude"))
        .env("FAKE_CLAUDE_DELAY_MS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let (Some(mut stdin), Some(stdout)) = (claude.stdin.take(), claude.stdout.take()) else {
        return Err("fake-claude's stdin and stdout aren't piped".into());
    };
    let turn = json!({"type": "user", "message": {"role": "user", "content": turn}});
    stdin.write_all(format!("{turn}\n").as_bytes())?;
    let mut messages = Vec::new();
    let mut results = 0;
    for line in BufReader::new(stdout).lines() {
        let v: Value = serde_json::from_str(&line?)?;
        if v["type"] == "stream_event" {
            continue;
        }
        results += usize::from(v["type"] == "result");
        messages.push(v);
        if results == until_results {
            break;
        }
    }
    drop(stdin);
    claude.wait()?;
    Ok(messages)
}

/// A message, for comparing sequences: its type (a system message's
/// subtype), after `sub` for the subagent's and `task` for Claude Code's
/// word on it.
fn shape(v: &Value, agent: &str) -> String {
    let what = match v["type"].as_str() {
        Some("system") => v["subtype"].as_str(),
        other => other,
    }
    .unwrap_or("?");
    if v["parent_tool_use_id"] == agent {
        format!("sub {what}")
    } else if v["tool_use_id"] == agent {
        format!("task {what}")
    } else {
        what.to_owned()
    }
}

#[test]
fn a_subagent_names_its_call_and_returns_its_report() -> Result<(), Box<dyn Error>> {
    let messages = subagent_messages("delegate a look around", 1)?;
    let call = messages
        .iter()
        .flat_map(|v| {
            v["message"]["content"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .find(|b| b["type"] == "tool_use" && b["name"] == "Agent")
        .ok_or("no Agent call")?;
    let agent = call["id"].as_str().ok_or("an Agent call without an id")?;
    let shapes: Vec<String> = messages
        .iter()
        .map(|v| shape(v, agent))
        .filter(|s| s.starts_with("sub") || s.starts_with("task") || s == "result")
        .collect();
    assert_eq!(
        shapes,
        [
            "task task_started",
            "sub user",
            "task task_progress",
            "sub assistant",
            "sub user",
            "sub assistant",
            "task task_progress",
            "sub assistant",
            "sub user",
            "task task_notification",
            "result"
        ]
    );
    let ended = messages
        .iter()
        .find(|v| v["subtype"] == "task_notification")
        .ok_or("no task_notification")?;
    assert_eq!(ended["status"], "completed");
    // The call returns the report, which Claude Code's copy has alone.
    let returned = messages
        .iter()
        .find(|v| v["message"]["content"][0]["tool_use_id"] == agent)
        .ok_or("the Agent call returned nothing")?;
    assert_eq!(returned["parent_tool_use_id"], Value::Null);
    assert_eq!(
        returned["tool_use_result"]["content"][0]["text"],
        ended["summary"]
    );
    assert!(
        returned["message"]["content"][0]["content"][0]["text"]
            .as_str()
            .is_some_and(|t| t.starts_with(ended["summary"].as_str().unwrap_or("?")))
    );
    Ok(())
}

#[test]
fn a_subagent_in_the_background_works_after_its_turn() -> Result<(), Box<dyn Error>> {
    let messages = subagent_messages("delegate it in the background", 2)?;
    let started = messages
        .iter()
        .find(|v| v["subtype"] == "task_started")
        .ok_or("no task_started")?;
    assert_eq!(started["is_backgrounded"], true);
    let agent = started["tool_use_id"]
        .as_str()
        .ok_or("a task without its call")?;
    let shapes: Vec<String> = messages
        .iter()
        .map(|v| shape(v, agent))
        .filter(|s| s.starts_with("sub") || s.starts_with("task") || s == "result")
        .collect();
    // Its call returns, the turn ends, and its steps come after.
    assert_eq!(
        shapes,
        [
            "task task_started",
            "sub user",
            "result",
            "task task_progress",
            "sub assistant",
            "sub user",
            "sub assistant",
            "task task_progress",
            "sub assistant",
            "sub user",
            "sub assistant",
            "task task_notification",
            "result"
        ]
    );
    let returned = messages
        .iter()
        .find(|v| v["message"]["content"][0]["tool_use_id"] == agent)
        .ok_or("the Agent call returned nothing")?;
    assert_eq!(returned["tool_use_result"]["isAsync"], true);
    // Its last words are its report.
    let last = messages
        .iter()
        .rfind(|v| v["parent_tool_use_id"] == agent)
        .ok_or("no last words")?;
    let ended = messages
        .iter()
        .find(|v| v["subtype"] == "task_notification")
        .ok_or("no task_notification")?;
    assert_eq!(last["message"]["content"][0]["text"], ended["summary"]);
    Ok(())
}

#[test]
fn get_settings_says_the_effort_it_works_at() -> Result<(), Box<dyn Error>> {
    let mut claude = Command::new(env!("CARGO_BIN_EXE_fake-claude"))
        .args(["--effort", "high"])
        .env("FAKE_CLAUDE_DELAY_MS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let (Some(mut stdin), Some(stdout)) = (claude.stdin.take(), claude.stdout.take()) else {
        return Err("fake-claude's stdin and stdout aren't piped".into());
    };
    let mut lines = BufReader::new(stdout).lines();
    let mut until = |done: &dyn Fn(&Value) -> bool| -> Result<Value, Box<dyn Error>> {
        loop {
            let v: Value = serde_json::from_str(&lines.next().ok_or("fake-claude exited")??)?;
            if done(&v) {
                return Ok(v);
            }
        }
    };
    // What it answers before any turn, then after turns that change it as
    // the Claude apps can.
    let mut efforts = Vec::new();
    for (n, turn) in [None, Some("effort max"), Some("effort none")]
        .into_iter()
        .enumerate()
    {
        if let Some(text) = turn {
            let turn = json!({"type": "user", "message": {"role": "user", "content": text}});
            stdin.write_all(format!("{turn}\n").as_bytes())?;
            until(&|v| v["type"] == "result")?;
        }
        let id = format!("req_{n}");
        let ask = json!({"type": "control_request", "request_id": id, "request": {"subtype": "get_settings"}});
        stdin.write_all(format!("{ask}\n").as_bytes())?;
        let answer =
            until(&|v| v["type"] == "control_response" && v["response"]["request_id"] == id)?;
        let applied = &answer["response"]["response"]["applied"];
        assert_eq!(applied["model"], "claude-fake-1");
        efforts.push(applied["effort"].clone());
    }
    drop(stdin);
    claude.wait()?;
    assert_eq!(efforts, [json!("high"), json!("max"), Value::Null]);
    Ok(())
}

#[test]
fn commits_pushes_and_watches_the_checks_in_one_turn() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let (home, repo) = (dir.path().join("home"), dir.path().join("widgets"));
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&repo)?;
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("HOME", &home)
            .status()
    };
    for args in [
        &["init", "-q", "-b", "fctl/1-test"][..],
        &["config", "user.name", "Ada"],
        &["config", "user.email", "ada@example.com"],
    ] {
        assert!(git(args)?.success());
    }
    let mut claude = Command::new(env!("CARGO_BIN_EXE_fake-claude"))
        .current_dir(&repo)
        .env("HOME", &home)
        .env("FAKE_CLAUDE_DELAY_MS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let (Some(mut stdin), Some(stdout)) = (claude.stdin.take(), claude.stdout.take()) else {
        return Err("fake-claude's stdin and stdout aren't piped".into());
    };
    let turn = json!({"type": "user", "message": {"role": "user", "content": "write notes.md, push it and watch 2"}});
    stdin.write_all(format!("{turn}\n").as_bytes())?;
    // Each tool call's command (or name), and when its result came.
    let mut calls = HashMap::new();
    let mut results = Vec::new();
    for line in BufReader::new(stdout).lines() {
        let v: Value = serde_json::from_str(&line?)?;
        let blocks = v["message"]["content"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        match v["type"].as_str() {
            Some("assistant") => {
                for b in blocks.iter().filter(|b| b["type"] == "tool_use") {
                    let what = b["input"]["command"].as_str().or(b["name"].as_str());
                    calls.insert(b["id"].clone(), what.unwrap_or_default().to_owned());
                }
            }
            Some("user") => {
                for b in blocks.iter().filter(|b| b["type"] == "tool_result") {
                    let call = calls.get(&b["tool_use_id"]).cloned().unwrap_or_default();
                    assert_eq!(b["is_error"], false, "{call}: {}", b["content"]);
                    results.push((call, Instant::now()));
                }
            }
            Some("result") => break,
            _ => {}
        }
    }
    drop(stdin);
    claude.wait()?;
    let calls: Vec<&str> = results.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(calls.len(), 5, "{calls:?}");
    assert_eq!(calls[0], "ls -la");
    assert_eq!(calls[1], "Write");
    assert!(calls[2].starts_with("git add notes.md && git commit"));
    assert_eq!(
        calls[3..],
        ["git push -u origin HEAD", "gh pr checks --watch"]
    );
    // The checks take their time after the push.
    assert!(results[4].1 - results[3].1 >= Duration::from_secs(2));
    // A bare repository stood in for GitHub, and has the branch.
    let remote = home.join(".fake-github/widgets.git");
    let has_branch = Command::new("git")
        .args(["--git-dir"])
        .arg(&remote)
        .args(["rev-parse", "--verify", "--quiet", "refs/heads/fctl/1-test"])
        .status()?;
    assert!(has_branch.success());
    Ok(())
}

#[test]
fn explores_in_a_run_of_calls_with_thoughts_among_them() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("README.md"),
        "# Widgets\n\nTODO: say what they are.\n",
    )?;
    let mut claude = Command::new(env!("CARGO_BIN_EXE_fake-claude"))
        .current_dir(dir.path())
        .env("FAKE_CLAUDE_DELAY_MS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let (Some(mut stdin), Some(stdout)) = (claude.stdin.take(), claude.stdout.take()) else {
        return Err("fake-claude's stdin and stdout aren't piped".into());
    };
    let turn = json!({"type": "user", "message": {"role": "user", "content": "explore"}});
    stdin.write_all(format!("{turn}\n").as_bytes())?;
    // What its assistant messages hold, in order, with their message's id.
    let mut said = Vec::new();
    let mut streamed_thoughts = 0;
    let mut results = HashMap::new();
    for line in BufReader::new(stdout).lines() {
        let v: Value = serde_json::from_str(&line?)?;
        if v["event"]["delta"]["type"] == "thinking_delta" {
            streamed_thoughts += 1;
        }
        let blocks = v["message"]["content"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        match v["type"].as_str() {
            Some("assistant") => {
                for b in blocks {
                    let what = match b["type"].as_str() {
                        Some("tool_use") => b["name"].as_str().unwrap_or_default().to_owned(),
                        Some(other) => other.to_owned(),
                        None => String::new(),
                    };
                    said.push((what, v["message"]["id"].clone(), b["id"].clone()));
                }
            }
            Some("user") => {
                for b in blocks.iter().filter(|b| b["type"] == "tool_result") {
                    results.insert(b["tool_use_id"].clone(), b["content"].clone());
                }
            }
            Some("result") => break,
            _ => {}
        }
    }
    drop(stdin);
    claude.wait()?;
    let what: Vec<&str> = said.iter().map(|(w, _, _)| w.as_str()).collect();
    assert_eq!(
        what,
        [
            "text", "Bash", "thinking", "Bash", "Bash", "Glob", "Read", "thinking", "Grep", "Bash",
            "text", "text"
        ]
    );
    assert!(streamed_thoughts > 1);
    // The listing and the read come at once, in one response.
    assert_eq!(said[5].1, said[6].1);
    let result = |i: usize| results.get(&said[i].2).cloned().unwrap_or_default();
    assert_eq!(result(5), "./README.md\n");
    assert_eq!(
        result(6),
        "     1\t# Widgets\n     2\t\n     3\tTODO: say what they are.\n"
    );
    assert!(
        result(8)
            .as_str()
            .is_some_and(|r| r.contains("README.md:3:TODO"))
    );
    Ok(())
}

/// A working directory with a git repository whose `origin` is a GitHub
/// URL, and a replay directory holding `replay` played 100 times faster.
fn replay_setup(replay: &str) -> Result<(tempfile::TempDir, std::path::PathBuf), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let (home, repo, replays) = (
        dir.path().join("home"),
        dir.path().join("shop"),
        dir.path().join("replays"),
    );
    for d in [&home, &repo, &replays] {
        std::fs::create_dir_all(d)?;
    }
    std::fs::create_dir_all(repo.join("lib"))?;
    std::fs::write(
        repo.join("lib/money.ts"),
        "export type Currency = \"SEK\";\n",
    )?;
    for args in [
        &["init", "-q", "-b", "fctl/231-add-danish"][..],
        &["config", "user.name", "Ada"],
        &["config", "user.email", "ada@example.com"],
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/romanescohq/shop.git",
        ],
        &["add", "-A"],
        &["commit", "-q", "-m", "Start"],
    ] {
        let ok = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("HOME", &home)
            .status()?
            .success();
        assert!(ok, "git {args:?}");
    }
    std::fs::write(replays.join("shop.toml"), replay)?;
    std::fs::write(replays.join("speed"), "100")?;
    Ok((dir, repo))
}

/// fake-claude with `--replays`, in `repo`.
fn replaying(repo: &std::path::Path) -> Result<std::process::Child, Box<dyn Error>> {
    let root = repo.parent().ok_or("no parent")?;
    Ok(Command::new(env!("CARGO_BIN_EXE_fake-claude"))
        .args(["--replays"])
        .arg(root.join("replays"))
        .current_dir(repo)
        .env("HOME", root.join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?)
}

#[test]
fn a_replay_plays_its_turn_and_makes_its_changes() -> Result<(), Box<dyn Error>> {
    let (_dir, repo) = replay_setup(
        r#"
match = "Add Danish to the shop"

[[turn]]
duration = "14m"
result = "Danish is in."
[[turn.step]]
think = "Prices first."
[[turn.step]]
tools = [
  { read = "lib/money.ts" },
  { grep = "formatPrice", path = "src", output = "src/price.ts:3:formatPrice" },
]
[[turn.step]]
edit = "lib/money.ts"
old = "\"SEK\""
new = "\"SEK\" | \"DKK\""
[[turn.step]]
write = "messages/da.json"
content = "{ \"cart\": \"Kurv\" }\n"
[[turn.step]]
commit = "Add Danish translations"
[[turn.step]]
pr = "Add Danish to the shop"
number = 1291
body = "Danish copy from SHOP-419."
[[turn.step]]
say = "Opened the pull request."
"#,
    )?;
    let mut claude = replaying(&repo)?;
    let (Some(mut stdin), Some(stdout)) = (claude.stdin.take(), claude.stdout.take()) else {
        return Err("fake-claude's stdin and stdout aren't piped".into());
    };
    let turn = json!({"type": "user", "message": {"role": "user", "content": "Tomás delegated SHOP-419: Add Danish to the shop for the Copenhagen launch"}});
    stdin.write_all(format!("{turn}\n").as_bytes())?;
    let mut calls = Vec::new();
    let mut outputs = HashMap::new();
    let mut thought = false;
    let mut result = Value::Null;
    for line in BufReader::new(stdout).lines() {
        let v: Value = serde_json::from_str(&line?)?;
        let blocks = v["message"]["content"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        match v["type"].as_str() {
            Some("assistant") => {
                thought |= blocks.iter().any(|b| b["type"] == "thinking");
                for b in blocks.iter().filter(|b| b["type"] == "tool_use") {
                    calls.push((b["id"].clone(), b["name"].clone(), b["input"].clone()));
                }
            }
            Some("user") => {
                for b in blocks.iter().filter(|b| b["type"] == "tool_result") {
                    assert_eq!(b["is_error"], false, "{}", b["content"]);
                    outputs.insert(b["tool_use_id"].clone(), b["content"].clone());
                }
            }
            Some("result") => {
                result = v;
                break;
            }
            _ => {}
        }
    }
    drop(stdin);
    claude.wait()?;
    assert!(thought);
    let names: Vec<&str> = calls.iter().filter_map(|(_, n, _)| n.as_str()).collect();
    assert_eq!(names, ["Read", "Grep", "Edit", "Write", "Bash", "Bash"]);
    // Files go by their absolute paths, and reads show the file.
    let (read, _, input) = &calls[0];
    let money = repo.join("lib/money.ts");
    assert_eq!(input["file_path"], money.display().to_string());
    assert_eq!(outputs[read], "     1\texport type Currency = \"SEK\";\n");
    // The edit, the new file and the commit are real.
    assert_eq!(
        std::fs::read_to_string(&money)?,
        "export type Currency = \"SEK\" | \"DKK\";\n"
    );
    assert!(repo.join("messages/da.json").is_file());
    let log = Command::new("git")
        .args(["log", "-1", "--format=%s%n%b"])
        .current_dir(&repo)
        .output()?;
    let log = String::from_utf8(log.stdout)?;
    assert!(log.starts_with("Add Danish translations\n"), "{log}");
    assert!(log.contains("Co-Authored-By: Claude"), "{log}");
    let (commit, _, input) = &calls[4];
    assert!(
        input["command"]
            .as_str()
            .is_some_and(|c| c.starts_with("git add -A && git commit -m")),
        "{input}"
    );
    assert!(
        outputs[commit]
            .as_str()
            .is_some_and(|o| o.contains("2 files changed")),
        "{}",
        outputs[commit]
    );
    // The pull request is the repository's on GitHub.
    let (pr, _, input) = &calls[5];
    assert!(
        input["command"]
            .as_str()
            .is_some_and(|c| c.starts_with("gh pr create --title \"Add Danish to the shop\""))
    );
    assert_eq!(
        outputs[pr],
        "https://github.com/romanescohq/shop/pull/1291\n"
    );
    // Its result says what it was given.
    assert_eq!(result["subtype"], "success");
    assert_eq!(result["result"], "Danish is in.");
    assert_eq!(result["duration_ms"], 14 * 60 * 1000);
    Ok(())
}

#[test]
fn a_held_call_works_until_it_is_interrupted() -> Result<(), Box<dyn Error>> {
    let (_dir, repo) = replay_setup(
        "match = \"Watch it\"\n[[turn]]\n[[turn.step]]\nsay = \"Watching.\"\n[[turn.step]]\nbash = \"gh pr checks 1291 --watch\"\nhold = true\n",
    )?;
    let mut claude = replaying(&repo)?;
    let (Some(mut stdin), Some(stdout)) = (claude.stdin.take(), claude.stdout.take()) else {
        return Err("fake-claude's stdin and stdout aren't piped".into());
    };
    let turn = json!({"type": "user", "message": {"role": "user", "content": "Watch it"}});
    stdin.write_all(format!("{turn}\n").as_bytes())?;
    let mut ticks = 0;
    let mut interrupted_at = None;
    let mut ended = Value::Null;
    for line in BufReader::new(stdout).lines() {
        let v: Value = serde_json::from_str(&line?)?;
        match v["type"].as_str() {
            // It ticks on, whatever the replay's speed, until interrupted.
            Some("tool_progress") => {
                ticks += 1;
                if ticks == 2 {
                    let interrupt = json!({"type": "control_request", "request_id": "req_1", "request": {"subtype": "interrupt"}});
                    stdin.write_all(format!("{interrupt}\n").as_bytes())?;
                    interrupted_at = Some(Instant::now());
                }
            }
            Some("result") => {
                ended = v;
                break;
            }
            _ => {}
        }
    }
    drop(stdin);
    claude.wait()?;
    assert!(interrupted_at.is_some_and(|at| at.elapsed() < Duration::from_secs(5)));
    assert_eq!(ended["subtype"], "error_during_execution");
    assert_eq!(ended["result"], "Interrupted");
    Ok(())
}

#[test]
fn a_replay_hits_the_usage_limit_and_later_turns_answer_their_messages()
-> Result<(), Box<dyn Error>> {
    let (_dir, repo) = replay_setup(
        "match = \"Match the box builder\"\n[[turn]]\n[[turn.step]]\nsay = \"Restyling.\"\n[[turn.step]]\nlimit = \"4h\"\n[[turn]]\nmatch = \"limit has reset\"\n[[turn.step]]\nsay = \"Back at it.\"\n",
    )?;
    let mut claude = replaying(&repo)?;
    let (Some(mut stdin), Some(stdout)) = (claude.stdin.take(), claude.stdout.take()) else {
        return Err("fake-claude's stdin and stdout aren't piped".into());
    };
    let turns: String = [
        "Match the box builder to the new illustrations",
        "Something else first",
        "The usage limit has reset. Continue where you left off.",
    ]
    .iter()
    .map(|text| {
        format!(
            "{}\n",
            json!({"type": "user", "message": {"role": "user", "content": text}})
        )
    })
    .collect();
    stdin.write_all(turns.as_bytes())?;
    let mut limit = Value::Null;
    let mut results = Vec::new();
    for line in BufReader::new(stdout).lines() {
        let v: Value = serde_json::from_str(&line?)?;
        match v["type"].as_str() {
            Some("rate_limit_event") if v["rate_limit_info"]["status"] == "rejected" => {
                limit = v["rate_limit_info"].clone();
            }
            Some("result") => {
                results.push(v["result"].clone());
                if results.len() == 3 {
                    break;
                }
            }
            _ => {}
        }
    }
    drop(stdin);
    claude.wait()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let resets = limit["resetsAt"].as_u64().ok_or("no reset time")?;
    assert!(
        (now + 4 * 3600 - 10..=now + 4 * 3600).contains(&resets),
        "{limit}"
    );
    assert_eq!(results[0], "Claude usage limit reached.");
    // A message the replay doesn't answer gets fake-claude's usual answer.
    assert!(results[1].as_str().is_some_and(|r| r.starts_with("Done.")));
    assert_eq!(results[2], "Back at it.");
    Ok(())
}
