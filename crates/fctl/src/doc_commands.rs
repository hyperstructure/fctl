//! Every `fctl` command the docs show parses as the CLI is now: its
//! commands and flags exist and its values read. Renaming a flag or a
//! command then can't leave the docs and their guides behind, and with
//! them whoever copies a command from one. It reads the shell code blocks
//! (```sh, or without a language) of `docs/*.md`, including the scripts
//! the guides show; ADRs keep the commands of their day.

use super::Cli;
use clap::Parser;
use clap::error::ErrorKind;
use std::path::PathBuf;

/// The shell code blocks of a Markdown text: ```sh ones, and those
/// without a language.
fn shell_blocks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut block: Option<(bool, String)> = None;
    for line in text.lines() {
        let fence = line.trim_start();
        if let Some(lang) = fence.strip_prefix("```") {
            match block.take() {
                Some((true, b)) => out.push(b),
                Some((false, _)) => {}
                None => block = Some((matches!(lang.trim(), "" | "sh"), String::new())),
            }
        } else if let Some((_, b)) = block.as_mut() {
            b.push_str(line);
            b.push('\n');
        }
    }
    out
}

/// The simple commands of a shell text, each as its words with quotes and
/// escapes taken off, as the shell would hand them to a program: split at
/// newlines, `;`, `&&`, `||`, `&` and pipes, without comments or
/// redirections. Those in `$(…)` come too, before the one they're in.
fn commands(text: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    lex(&chars, &mut out);
    out
}

/// Where the `$(` at `at` closes: its `)`, or the end.
fn closing(chars: &[char], at: usize) -> usize {
    let mut depth = 0;
    for (i, c) in chars.iter().enumerate().skip(at + 1) {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    chars.len()
}

fn lex(chars: &[char], out: &mut Vec<Vec<String>>) {
    let n = chars.len();
    let mut words: Vec<String> = Vec::new();
    let mut word: Option<String> = None;
    let end_word = |word: &mut Option<String>, words: &mut Vec<String>| {
        if let Some(w) = word.take() {
            words.push(w);
        }
    };
    let end_command =
        |word: &mut Option<String>, words: &mut Vec<String>, out: &mut Vec<Vec<String>>| {
            end_word(word, words);
            if !words.is_empty() {
                out.push(std::mem::take(words));
            }
        };
    let mut i = 0;
    while i < n {
        match chars[i] {
            '\\' if i + 1 < n => {
                // A line that goes on.
                if chars[i + 1] != '\n' {
                    word.get_or_insert_with(String::new).push(chars[i + 1]);
                }
                i += 2;
            }
            '\'' => {
                let end = (i + 1..n).find(|&j| chars[j] == '\'').unwrap_or(n);
                word.get_or_insert_with(String::new)
                    .extend(&chars[i + 1..end]);
                i = end + 1;
            }
            '"' => {
                let w = word.get_or_insert_with(String::new);
                i += 1;
                while i < n && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < n {
                        w.push(chars[i + 1]);
                        i += 2;
                    } else if chars[i] == '$' && chars.get(i + 1) == Some(&'(') {
                        let end = closing(chars, i);
                        lex(&chars[i + 2..end], out);
                        w.extend(&chars[i..(end + 1).min(n)]);
                        i = end + 1;
                    } else {
                        w.push(chars[i]);
                        i += 1;
                    }
                }
                i += 1;
            }
            '$' if chars.get(i + 1) == Some(&'(') => {
                let end = closing(chars, i);
                lex(&chars[i + 2..end], out);
                word.get_or_insert_with(String::new)
                    .extend(&chars[i..(end + 1).min(n)]);
                i = end + 1;
            }
            '#' if word.is_none() => {
                i = (i..n).find(|&j| chars[j] == '\n').unwrap_or(n);
            }
            ' ' | '\t' => {
                end_word(&mut word, &mut words);
                i += 1;
            }
            '\n' | ';' | '&' | '|' | '(' | ')' => {
                end_command(&mut word, &mut words, out);
                i += 1;
            }
            '<' | '>' => {
                // A redirection, `2>&1` or `> file`: neither it nor its
                // file is one of the program's words.
                if word
                    .as_deref()
                    .is_some_and(|w| w.chars().all(|c| c.is_ascii_digit()))
                {
                    word = None;
                }
                end_word(&mut word, &mut words);
                while i < n && matches!(chars[i], '<' | '>' | '&') {
                    i += 1;
                }
                while i < n && matches!(chars[i], ' ' | '\t') {
                    i += 1;
                }
                while i < n && !matches!(chars[i], ' ' | '\t' | '\n' | ';' | '&' | '|' | ')') {
                    i += 1;
                }
            }
            c => {
                word.get_or_insert_with(String::new).push(c);
                i += 1;
            }
        }
    }
    end_command(&mut word, &mut words, out);
}

/// A variable set for the one command (`FCTL_SERVER=… fctl …`).
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// What fctl gets, its own name first, where `words` run it: after the
/// variables set for it, a prompt's `$`, `sudo -u fctl`, `runuser -u fctl
/// --` and `env`, by its path (`/home/fctl/.local/bin/fctl`), and as
/// `cargo run -p fctl --`.
fn fctl_args(words: &[String]) -> Option<Vec<String>> {
    const CARGO_RUN: [&str; 5] = ["cargo", "run", "-p", "fctl", "--"];
    let mut rest = words;
    loop {
        let first = rest.first()?.as_str();
        if is_assignment(first) || matches!(first, "$" | "{" | "--" | "exec" | "env") {
            rest = &rest[1..];
        } else if matches!(first, "sudo" | "runuser") {
            rest = &rest[1..];
            while let Some(flag) = rest.first().filter(|w| w.starts_with('-') && *w != "--") {
                let skip = if matches!(flag.as_str(), "-u" | "-g") {
                    2
                } else {
                    1
                };
                rest = rest.get(skip..).unwrap_or_default();
            }
        } else if rest.len() >= CARGO_RUN.len() && rest.iter().zip(CARGO_RUN).all(|(w, c)| w == c) {
            rest = &rest[CARGO_RUN.len() - 1..];
            break;
        } else {
            let program = rest.first()?;
            if program == "fctl" || program.ends_with("/fctl") {
                break;
            }
            return None;
        }
    }
    // A script's own arguments ("$@"), none here.
    let given = rest[1..]
        .iter()
        .filter(|w| !matches!(w.as_str(), "$@" | "$*"));
    Some(
        std::iter::once("fctl".to_owned())
            .chain(given.cloned())
            .collect(),
    )
}

fn docs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs")
}

#[test]
fn every_fctl_command_in_the_docs_parses() -> std::io::Result<()> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(docs())? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "md") {
            files.push(path);
        }
    }
    files.sort();
    let mut seen = 0;
    let mut wrong = Vec::new();
    for file in &files {
        let name = file.file_name().unwrap_or_default().to_string_lossy();
        let text = std::fs::read_to_string(file)?;
        for block in shell_blocks(&text) {
            for args in commands(&block).iter().filter_map(|w| fctl_args(w)) {
                seen += 1;
                let Err(e) = Cli::try_parse_from(&args) else {
                    continue;
                };
                if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                    continue;
                }
                let why = e.render().to_string();
                let why = why.lines().next().unwrap_or_default().to_owned();
                wrong.push(format!("docs/{name}: {}\n  {why}", args.join(" ")));
            }
        }
    }
    assert!(seen > 500, "found only {seen} fctl commands in docs/");
    assert!(
        wrong.is_empty(),
        "{} of the docs' {seen} fctl commands don't parse:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    Ok(())
}

#[test]
fn reads_commands_as_the_shell_does() {
    let words = |text: &str| -> Vec<Vec<String>> { commands(text) };
    let owned = |list: &[&[&str]]| -> Vec<Vec<String>> {
        list.iter()
            .map(|c| c.iter().map(|w| (*w).to_owned()).collect())
            .collect()
    };
    assert_eq!(
        words(
            "fctl run -p web \"Fix the login\" # a comment\n\
             fctl host install --capacity 3 \\\n  --memory 8g 2>&1 | tee log\n\
             vault write auth/fctl/login jwt=\"$(fctl id-token -a 'https://vault')\"\n\
             FCTL_SERVER=https://fctl.acme.dev sudo -u fctl fctl whoami > out && echo done"
        ),
        owned(&[
            &["fctl", "run", "-p", "web", "Fix the login"],
            &[
                "fctl",
                "host",
                "install",
                "--capacity",
                "3",
                "--memory",
                "8g"
            ],
            &["tee", "log"],
            &["fctl", "id-token", "-a", "https://vault"],
            &[
                "vault",
                "write",
                "auth/fctl/login",
                "jwt=$(fctl id-token -a 'https://vault')"
            ],
            &[
                "FCTL_SERVER=https://fctl.acme.dev",
                "sudo",
                "-u",
                "fctl",
                "fctl",
                "whoami"
            ],
            &["echo", "done"],
        ])
    );
    let args = |text: &str| commands(text).first().and_then(|w| fctl_args(w));
    assert_eq!(
        args("FCTL_SERVER=x sudo -u fctl /home/fctl/.local/bin/fctl host doctor"),
        Some(vec!["fctl".into(), "host".into(), "doctor".into()])
    );
    assert_eq!(
        args("runuser -u fctl -- env XDG_RUNTIME_DIR=/run/user/1001 fctl host status \"$@\""),
        Some(vec!["fctl".into(), "host".into(), "status".into()])
    );
    assert_eq!(
        args("FCTL_CONFIG_DIR=.fctl-dev/host cargo run -p fctl -- host doctor"),
        Some(vec!["fctl".into(), "host".into(), "doctor".into()])
    );
    assert_eq!(
        args("$ fctl whoami"),
        Some(vec!["fctl".into(), "whoami".into()])
    );
    assert_eq!(args("echo fctl run"), None);
    assert_eq!(args("docker stop fctl"), None);
    assert_eq!(args("fctlx run"), None);
    assert_eq!(
        shell_blocks("```sh\nfctl a\n```\n```json\n{}\n```\n```\nfctl b\n```\n"),
        ["fctl a\n", "fctl b\n"]
    );
}
