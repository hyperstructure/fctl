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
