//! `fctl account two-factor …`: codes from an authenticator app after your
//! password (and after Google, when you or the instance choose that), and
//! recovery codes for when the app is gone.

use crate::prompt;
use crate::style;
use anyhow::Result;
use clap::Subcommand;
use clap::builder::BoolishValueParser;
use fctl_client::Api;
use fctl_client::http::Error as ClientError;
use fctl_proto::ErrorCode;
use fctl_proto::api::{
    AuthenticatorSetup, ConfirmAuthenticatorRequest, TwoFactor, UpdateTwoFactorRequest,
};
use std::io::{BufRead, IsTerminal};

#[derive(Subcommand)]
pub enum TwoFactorCmd {
    /// Show your two-factor sign-in, and what the instance requires.
    Status,
    /// Set up an authenticator app: scan the QR code this prints, or type
    /// in the key, then enter the code the app shows. It replaces the app
    /// you have, if any. Without a terminal (or with --json) it prints the
    /// setup for `confirm`.
    #[command(
        after_help = "Any authenticator app works: 1Password, Google Authenticator, Microsoft Authenticator, Authy,\nor the passwords app on your phone.\n\nWithout a terminal:\n  fctl account two-factor enable --json > setup.json\n  fctl account two-factor confirm --setup \"$(jq -r .setup setup.json)\" --code 123456"
    )]
    Enable {
        /// When you replace the app you have: a code from it, or a recovery
        /// code (asked for when left out).
        #[arg(long)]
        code: Option<String>,
    },
    /// Confirm an authenticator app that `enable --json` set up, with the
    /// code it shows.
    Confirm {
        /// The `setup` that `enable --json` printed.
        #[arg(long)]
        setup: String,
        /// The code the app shows now.
        #[arg(long)]
        code: String,
        /// Read your password from the first line of stdin, when `enable`
        /// said the instance asks for it (you signed in more than 15
        /// minutes ago).
        #[arg(long)]
        password_stdin: bool,
    },
    /// Turn two-factor sign-in off (not while the instance requires it).
    Disable {
        /// A code from your authenticator app, or a recovery code (asked
        /// for when left out).
        #[arg(long)]
        code: Option<String>,
    },
    /// Get ten new recovery codes; the old ones stop working.
    RecoveryCodes {
        /// A code from your authenticator app, or a recovery code (asked
        /// for when left out).
        #[arg(long)]
        code: Option<String>,
    },
    /// Choose whether Google sign-ins ask you for a code too.
    Google {
        #[arg(value_name = "ON|OFF", action = clap::ArgAction::Set, value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        on: bool,
    },
}

pub async fn run(api: &Api, cmd: TwoFactorCmd, json: bool) -> Result<()> {
    match cmd {
        TwoFactorCmd::Status => {
            let status = api.two_factor().await?;
            if json {
                return style::print_json(&status);
            }
            print_status(&status);
        }
        TwoFactorCmd::Enable { code } => {
            // Replacing an app takes a code from it, as turning it off does.
            let replacing = api.two_factor().await?.authenticator_since.is_some();
            let code = match code {
                Some(c) => Some(c),
                None if replacing => Some(prompt::read_line(
                    "You have an authenticator app. A code from it, or a recovery code, to replace it: ",
                    "--code",
                )?),
                None => None,
            };
            let setup = api.start_authenticator(code.as_deref()).await?;
            if json {
                return style::print_json(&setup);
            }
            if !std::io::stdin().is_terminal() {
                print_setup(&setup);
                println!();
                println!("Then confirm it with the code the app shows:");
                println!(
                    "  fctl account two-factor confirm --setup {} --code <code>{}",
                    setup.setup,
                    if setup.password_required {
                        " --password-stdin"
                    } else {
                        ""
                    }
                );
                return Ok(());
            }
            print_setup(&setup);
            println!();
            let password = if setup.password_required {
                Some(prompt::read_secret(
                    "Your password (it's been a while since you signed in): ",
                )?)
            } else {
                None
            };
            let mut tries = 0;
            let added = loop {
                let code = prompt::read_line("Code from the app: ", "the code")?;
                let req = ConfirmAuthenticatorRequest {
                    setup: setup.setup.clone(),
                    code,
                    password: password.clone(),
                };
                match api.confirm_authenticator(&req).await {
                    Ok(added) => break added,
                    Err(ClientError::Api(e))
                        if e.code == ErrorCode::Validation
                            && e.field.as_deref() == Some("code")
                            && tries < 2 =>
                    {
                        tries += 1;
                        println!("{}", style::amber(&e.message));
                    }
                    Err(e) => return Err(e.into()),
                }
            };
            println!(
                "{} Two-factor sign-in is on: signing in with your password now also takes a code.",
                style::accent("✓")
            );
            match &added.recovery_codes {
                Some(codes) => print_recovery_codes(codes),
                None => println!(
                    "{}",
                    style::dim("The new app replaced the old one. Your recovery codes still work.")
                ),
            }
        }
        TwoFactorCmd::Confirm {
            setup,
            code,
            password_stdin,
        } => {
            let password = password_stdin.then(read_stdin_line).transpose()?;
            let added = api
                .confirm_authenticator(&ConfirmAuthenticatorRequest {
                    setup,
                    code,
                    password,
                })
                .await?;
            if json {
                return style::print_json(&added);
            }
            match &added.recovery_codes {
                Some(codes) => {
                    println!("{} Two-factor sign-in is on.", style::accent("✓"));
                    print_recovery_codes(codes);
                }
                None => println!(
                    "{} The new authenticator app is in use. Your recovery codes still work.",
                    style::accent("✓")
                ),
            }
        }
        TwoFactorCmd::Disable { code } => {
            let code = code_or_ask(code)?;
            let me = api.remove_authenticator(&code).await?;
            if json {
                return style::print_json(&me.two_factor);
            }
            println!(
                "{} Two-factor sign-in is off: your password alone signs you in.",
                style::accent("✓")
            );
        }
        TwoFactorCmd::RecoveryCodes { code } => {
            let code = code_or_ask(code)?;
            let codes = api.new_recovery_codes(&code).await?;
            if json {
                return style::print_json(&codes);
            }
            println!(
                "{} New recovery codes. The old ones no longer work.",
                style::accent("✓")
            );
            print_recovery_codes(&codes.codes);
        }
        TwoFactorCmd::Google { on } => {
            let me = api
                .update_two_factor(&UpdateTwoFactorRequest { google: Some(on) })
                .await?;
            if json {
                return style::print_json(&me.two_factor);
            }
            let t = &me.two_factor;
            println!(
                "{} {}",
                style::accent("✓"),
                match (on, t.google_required) {
                    (_, true) =>
                        "Google sign-ins ask you for a code: the instance asks it of everyone.",
                    (true, false) => "Google sign-ins ask you for a code too.",
                    (false, false) => "Google sign-ins don't ask you for a code.",
                }
            );
            if on && t.authenticator_since.is_none() {
                println!(
                    "{}",
                    style::dim(
                        "They start asking once you set up an authenticator app: fctl account two-factor enable"
                    )
                );
            }
        }
    }
    Ok(())
}

/// What `whoami` and `status` say about two-factor sign-in, one line.
pub fn summary(t: &TwoFactor) -> String {
    match t.authenticator_since {
        Some(_) => format!(
            "on (authenticator app, {} recovery code{} left)",
            t.recovery_codes,
            if t.recovery_codes == 1 { "" } else { "s" }
        ),
        None if t.setup_required => {
            "required and not set up (run `fctl account two-factor enable`)".to_owned()
        }
        None if t.on => "on (your passkeys)".to_owned(),
        None => "off".to_owned(),
    }
}

fn print_status(t: &TwoFactor) {
    let state = match t.authenticator_since {
        Some(_) => style::accent("on"),
        None if t.setup_required => style::red("required, not set up"),
        None if t.on => style::accent("on, with your passkeys"),
        None => style::dim("off"),
    };
    println!("Two-factor sign-in  {state}");
    if let Some(since) = t.authenticator_since {
        let used = t
            .authenticator_used_at
            .map(|u| format!(", last used {}", style::ago(u)))
            .unwrap_or_default();
        println!("Authenticator app   set up {}{used}", style::ago(since));
        let left = format!("{} of 10 left", t.recovery_codes);
        println!(
            "Recovery codes      {}",
            if t.recovery_codes <= fctl_domain::two_factor::FEW_RECOVERY_CODES {
                style::amber(&format!(
                    "{left}: get new ones with `fctl account two-factor recovery-codes`"
                ))
            } else {
                left
            }
        );
    }
    let google = if t.google_required {
        "ask for a code (the instance asks it of everyone)"
    } else if t.google {
        "ask for a code too"
    } else {
        "don't ask for a code"
    };
    println!("Google sign-ins     {google}");
    if t.setup_required {
        println!(
            "{}",
            style::amber(
                "This instance requires two-factor sign-in: set it up with `fctl account two-factor enable`."
            )
        );
    } else if t.required {
        println!(
            "{}",
            style::dim("This instance requires two-factor sign-in, so it stays on.")
        );
    } else if t.authenticator_since.is_none() {
        println!(
            "{}",
            style::dim("Turn it on with `fctl account two-factor enable`.")
        );
    }
}

/// The QR code and the key, for the app to take.
fn print_setup(setup: &AuthenticatorSetup) {
    if style::colors() {
        println!("Scan this with your authenticator app:");
        println!();
        print_qr(&setup.qr);
        println!();
        println!(
            "Or type in this key: {}",
            style::bold(&fctl_domain::totp::grouped(&setup.secret))
        );
    } else {
        println!(
            "Type this key into your authenticator app: {}",
            fctl_domain::totp::grouped(&setup.secret)
        );
        println!("Or give it this link: {}", setup.uri);
    }
}

/// Prints QR rows dark on a light ground, whatever the terminal's colors.
fn print_qr(rows: &[String]) {
    for line in qr_lines(rows) {
        println!("  \x1b[30;107m{line}\x1b[0m");
    }
}

/// Modules around the code that stay light, for scanners to find it.
const QUIET: usize = 4;

/// QR rows as half blocks, two rows of modules to a line, with the quiet
/// zone around them: `▀` is a dark module over a light one.
fn qr_lines(rows: &[String]) -> Vec<String> {
    let size = rows.len() + 2 * QUIET;
    let dark = |y: usize, x: usize| -> bool {
        y >= QUIET
            && x >= QUIET
            && rows
                .get(y - QUIET)
                .and_then(|r| r.as_bytes().get(x - QUIET))
                == Some(&b'1')
    };
    (0..size)
        .step_by(2)
        .map(|y| {
            (0..size)
                .map(|x| match (dark(y, x), dark(y + 1, x)) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => ' ',
                })
                .collect()
        })
        .collect()
}

fn print_recovery_codes(codes: &[String]) {
    println!();
    println!(
        "{}",
        style::bold("Recovery codes: each one signs you in once if you lose your phone.")
    );
    println!(
        "{}",
        style::amber(
            "Save them somewhere safe now, such as your password manager: they are shown once."
        )
    );
    println!();
    for pair in codes.chunks(2) {
        println!("  {}", pair.join("    "));
    }
    println!();
}

fn code_or_ask(code: Option<String>) -> Result<String> {
    match code {
        Some(c) => Ok(c),
        None => prompt::read_line(
            "A code from your authenticator app, or a recovery code: ",
            "--code",
        ),
    }
}

fn read_stdin_line() -> Result<String> {
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_blocks_hold_two_rows_of_modules() {
        let rows = fctl_domain::qr::rows(
            "otpauth://totp/fctl:ada%40example.com?secret=JBSWY3DPEHPK3PXP&issuer=fctl",
        )
        .unwrap_or_default();
        let lines = qr_lines(&rows);
        let size = rows.len() + 2 * QUIET;
        assert_eq!(lines.len(), size.div_ceil(2));
        // Reading the blocks back gives the modules, quiet zone and all.
        for (y, line) in lines.iter().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            assert_eq!(chars.len(), size);
            for (x, c) in chars.iter().enumerate() {
                let (top, bottom) = match c {
                    '█' => (true, true),
                    '▀' => (true, false),
                    '▄' => (false, true),
                    _ => (false, false),
                };
                let module = |row: usize| {
                    row >= QUIET
                        && x >= QUIET
                        && rows
                            .get(row - QUIET)
                            .and_then(|r| r.as_bytes().get(x - QUIET))
                            == Some(&b'1')
                };
                assert_eq!(top, module(2 * y), "({x}, {})", 2 * y);
                assert_eq!(bottom, module(2 * y + 1), "({x}, {})", 2 * y + 1);
            }
        }
        // The quiet zone is light.
        assert!(lines[0].chars().all(|c| c == ' '));
    }
}
