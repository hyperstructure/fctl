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
fn background_work_ends_in_a_turn_of_its_own() -> Result<(), Box<dyn Error>> {
    let mut claude = Command::new(env!("CARGO_BIN_EXE_fake-claude"))
        .env("FAKE_CLAUDE_DELAY_MS", "1")
        .env("CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let (Some(mut stdin), Some(stdout)) = (claude.stdin.take(), claude.stdout.take()) else {
        return Err("fake-claude's stdin and stdout aren't piped".into());
    };
    let turn = json!({"type": "user", "message": {"role": "user", "content": "Run it in the background 1"}});
    stdin.write_all(format!("{turn}\n").as_bytes())?;
    // The turn, the command's end, and the turn that takes its result.
    let mut seen = Vec::new();
    let mut task = None;
    for line in BufReader::new(stdout).lines() {
        let v: Value = serde_json::from_str(&line?)?;
        let what = match (v["type"].as_str(), v["subtype"].as_str()) {
            (Some("system"), Some("session_state_changed")) => {
                v["state"].as_str().map(str::to_owned)
            }
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
        if seen.len() == 8 {
            break;
        }
    }
    drop(stdin);
    claude.wait()?;
    assert_eq!(
        seen,
        [
            "running",
            "task_started",
            "result",
            "idle",
            "task_notification",
            "running",
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
