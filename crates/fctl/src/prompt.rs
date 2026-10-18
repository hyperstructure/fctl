//! Reading what people type in the terminal.

use anyhow::{Result, bail};
use std::io::{BufRead, IsTerminal, Write};

/// Asks for a line without showing what is typed (passwords, tokens).
pub fn read_secret(prompt: &str) -> Result<String> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let tty = std::io::stdin().is_terminal();
    if tty {
        let _ = std::process::Command::new("stty").arg("-echo").status();
    }
    let mut line = String::new();
    let r = std::io::stdin().lock().read_line(&mut line);
    if tty {
        let _ = std::process::Command::new("stty").arg("echo").status();
        println!();
    }
    r?;
    Ok(line.trim().to_owned())
}

/// Asks for a line, in a terminal; `what` names the flag that answers it
/// without one.
pub fn read_line(prompt: &str, what: &str) -> Result<String> {
    if !std::io::stdin().is_terminal() {
        bail!("pass {what}: there is no terminal to ask in");
    }
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_owned())
}
