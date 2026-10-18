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
    Enable,
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
        #[arg(value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
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
        TwoFactorCmd::Enable => {
            let setup = api.start_authenticator().await?;
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
            println!("{} Two-factor sign-in is on.", style::accent("✓"));
            if let Some(codes) = &added.recovery_codes {
                print_recovery_codes(codes);
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
            "on: authenticator app, {} recovery code{} left",
            t.recovery_codes,
            if t.recovery_codes == 1 { "" } else { "s" }
        ),
        None if t.setup_required => {
            "required, not set up: run `fctl account two-factor enable`".to_owned()
        }
        None => "off".to_owned(),
    }
}

fn print_status(t: &TwoFactor) {
    let state = match t.authenticator_since {
        Some(_) => style::accent("on"),
        None if t.setup_required => style::red("required, not set up"),
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
    if t.required {
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

/// Prints QR rows with half blocks: each line holds two rows of modules,
/// dark on a light ground with a quiet zone around, whatever the terminal's
/// colors.
fn print_qr(rows: &[String]) {
    const QUIET: usize = 2;
    let n = rows.len();
    let dark = |y: usize, x: usize| -> bool {
        y >= QUIET
            && x >= QUIET
            && rows
                .get(y - QUIET)
                .and_then(|r| r.as_bytes().get(x - QUIET))
                == Some(&b'1')
    };
    let size = n + 2 * QUIET;
    for y in (0..size).step_by(2) {
        let mut line = String::from("  \x1b[30;107m");
        for x in 0..size {
            line.push(match (dark(y, x), y + 1 < size && dark(y + 1, x)) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        line.push_str("\x1b[0m");
        println!("{line}");
    }
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
