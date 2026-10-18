//! fake-claude driven as fctl-agent drives Claude Code: user turns on its
//! stdin, messages on its stdout.

use serde_json::{Value, json};
use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

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
