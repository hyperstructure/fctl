//! Two-factor sign-in: who needs it, which sign-ins ask for it, recovery
//! codes, and how long sign-ins wait after wrong codes. The codes
//! themselves are [`crate::totp`].
//!
//! A sign-in with a password asks for the second factor once the account
//! has an authenticator app. A Google sign-in asks only when the person
//! chose that, or the instance's admins did for everyone: Google's own
//! sign-in, with its two-step verification, protects it otherwise. When the
//! instance requires two-factor, every account that signs in with a
//! password needs a second factor, and with Google sign-ins asking too,
//! every account does: an authenticator app, or a passkey.
//!
//! Passkeys sign in on their own: the device checks who it is (fingerprint,
//! face, PIN), so they are two factors in one. Where two-factor is
//! required they also count as the second factor, after a password or
//! Google; elsewhere having one changes nothing about other sign-ins.

use crate::random_bytes;
use crate::tokens::{USER_CODE_ALPHABET, hash_secret};
use fctl_proto::api::InstanceSettings;

/// Recovery codes an account gets at a time.
pub const RECOVERY_CODES: usize = 10;
/// Characters in a recovery code, shown in two groups (`KFTWQ-XPDMB`).
const RECOVERY_CODE_LEN: usize = 10;
/// Unused recovery codes at or below which Settings suggests new ones.
pub const FEW_RECOVERY_CODES: u32 = 3;
/// Wrong codes in a row after which sign-ins wait.
pub const FREE_FAILURES: u32 = 5;
/// The longest a sign-in waits after wrong codes.
const MAX_LOCKOUT_MS: i64 = 60 * 60 * 1000;
/// How long a sign-in waits for its second factor.
pub const CHALLENGE_TTL_MS: i64 = 10 * 60 * 1000;
/// How long a new authenticator app's secret waits for its first code.
pub const SETUP_TTL_MS: i64 = 15 * 60 * 1000;
/// Changing two-factor takes the password, unless the sign-in it is done
/// with is younger than this.
pub const FRESH_SIGN_IN_MS: i64 = 15 * 60 * 1000;

/// How someone signs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Password,
    Google,
}

/// What an account has to prove it's them beyond a password or Google.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Factors {
    /// An authenticator app: two-factor sign-in is on.
    pub authenticator: bool,
    /// At least one passkey.
    pub passkeys: bool,
}

impl Factors {
    /// An authenticator app or a passkey.
    pub fn any(self) -> bool {
        self.authenticator || self.passkeys
    }
}

/// Whether the instance requires two-factor of an account: when it signs in
/// with a password, or at all once Google sign-ins ask for it too.
pub fn required(settings: &InstanceSettings, has_password: bool) -> bool {
    settings.require_two_factor && (has_password || settings.two_factor_google)
}

/// The instance's settings as they apply to an account that is a member of
/// a workspace requiring two-factor sign-in (`in_workspace`): as if the
/// instance required it. Its sign-ins then ask for the second factor, a
/// passkey included, and its last factor can't go; only the instance's own
/// requirement closes the whole account ([`setup_required`]).
pub fn with_workspaces(settings: &InstanceSettings, in_workspace: bool) -> InstanceSettings {
    InstanceSettings {
        require_two_factor: settings.require_two_factor || in_workspace,
        ..settings.clone()
    }
}

/// Whether a workspace requires two-factor sign-in of its members: its
/// admins chose that (`own`), or the instance requires it, which it then
/// does of every workspace, and their admins can't turn it off
/// ([`crate::workspace::check_two_factor`]). The instance's requirement
/// closes the accounts it applies to that have no second factor
/// ([`setup_required`]), so a workspace closes to them only by its own
/// ([`workspace_blocked`]).
pub fn workspace_requires(settings: &InstanceSettings, own: bool) -> bool {
    own || settings.require_two_factor
}

/// Whether a workspace that requires two-factor sign-in is closed to an
/// account: the requirement applies to it as the instance's would (it signs
/// in with a password, or Google sign-ins ask for codes), and it has
/// neither an authenticator app nor a passkey.
pub fn workspace_blocked(
    settings: &InstanceSettings,
    has_password: bool,
    factors: Factors,
) -> bool {
    (has_password || settings.two_factor_google) && !factors.any()
}

/// Whether an account the instance requires two-factor of has neither an
/// authenticator app nor a passkey: until it has one, it can do nothing else.
pub fn setup_required(settings: &InstanceSettings, has_password: bool, factors: Factors) -> bool {
    required(settings, has_password) && !(factors.authenticator || factors.passkeys)
}

/// Whether sign-ins with a password (and Google, when it asks) wait for a
/// second factor: the account has an authenticator app, or the instance
/// requires one and the account has a passkey for it.
pub fn on(settings: &InstanceSettings, has_password: bool, factors: Factors) -> bool {
    factors.authenticator || (factors.passkeys && required(settings, has_password))
}

/// Whether Google sign-ins ask this person for a code: they or the
/// instance chose that.
pub fn asks_on_google(settings: &InstanceSettings, own_choice: bool) -> bool {
    own_choice || settings.two_factor_google
}

/// Whether a sign-in waits for the second factor: two-factor is [`on`] for
/// the account, and this way of signing in asks for it.
pub fn asks(
    settings: &InstanceSettings,
    via: Via,
    has_password: bool,
    factors: Factors,
    own_google: bool,
) -> bool {
    on(settings, has_password, factors)
        && match via {
            Via::Password => true,
            Via::Google => asks_on_google(settings, own_google),
        }
}

/// Whether the authenticator app can go: not while the instance requires
/// two-factor of this account and no passkey would take its place.
pub fn can_turn_off(settings: &InstanceSettings, has_password: bool, factors: Factors) -> bool {
    !required(settings, has_password) || factors.passkeys
}

/// Whether an account's last passkey can go: not while the instance
/// requires two-factor of it and it has no authenticator app.
pub fn can_remove_last_passkey(
    settings: &InstanceSettings,
    has_password: bool,
    factors: Factors,
) -> bool {
    !required(settings, has_password) || factors.authenticator
}

/// How long sign-ins wait after `failures` wrong codes in a row: not at all
/// for fewer than [`FREE_FAILURES`], then a minute, doubling with each
/// further one, up to an hour. A right code starts the count over.
pub fn lockout_ms(failures: u32) -> Option<i64> {
    let over = failures.checked_sub(FREE_FAILURES)?;
    Some((60_000_i64 << over.min(6)).min(MAX_LOCKOUT_MS))
}

/// A new recovery code: ten characters without vowels or look-alikes, in
/// two groups (`KFTWQ-XPDMB`), about 43 bits.
pub fn new_recovery_code() -> String {
    let mut chars = Vec::with_capacity(RECOVERY_CODE_LEN);
    // Rejection sampling keeps every character equally likely.
    let limit = (256 / USER_CODE_ALPHABET.len() * USER_CODE_ALPHABET.len()) as u8;
    while chars.len() < RECOVERY_CODE_LEN {
        for b in random_bytes::<16>() {
            if b < limit && chars.len() < RECOVERY_CODE_LEN {
                chars.push(USER_CODE_ALPHABET[usize::from(b) % USER_CODE_ALPHABET.len()]);
            }
        }
    }
    let (a, b) = chars.split_at(RECOVERY_CODE_LEN / 2);
    format!(
        "{}-{}",
        String::from_utf8_lossy(a),
        String::from_utf8_lossy(b)
    )
}

/// A set of [`RECOVERY_CODES`] new recovery codes.
pub fn new_recovery_codes() -> Vec<String> {
    let mut codes: Vec<String> = Vec::with_capacity(RECOVERY_CODES);
    while codes.len() < RECOVERY_CODES {
        let code = new_recovery_code();
        if !codes.contains(&code) {
            codes.push(code);
        }
    }
    codes
}

/// What someone typed (`kftwq xpdmb`, `KFTWQXPDMB`), as recovery codes are
/// hashed (`KFTWQXPDMB`), if it can be one.
pub fn normalize_recovery_code(typed: &str) -> Option<String> {
    let chars: String = typed
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (chars.len() == RECOVERY_CODE_LEN && chars.bytes().all(|c| USER_CODE_ALPHABET.contains(&c)))
        .then_some(chars)
}

/// What the database keeps of a recovery code: the SHA-256 of its
/// normalized form, so it is found however it was typed.
pub fn hash_recovery_code(code: &str) -> Option<String> {
    normalize_recovery_code(code).map(|c| hash_secret(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(require: bool, google: bool) -> InstanceSettings {
        InstanceSettings {
            require_two_factor: require,
            two_factor_google: google,
            ..Default::default()
        }
    }

    const APP: Factors = Factors {
        authenticator: true,
        passkeys: false,
    };
    const PASSKEY: Factors = Factors {
        authenticator: false,
        passkeys: true,
    };
    const NONE: Factors = Factors {
        authenticator: false,
        passkeys: false,
    };

    #[test]
    fn workspaces_that_require_it_close_to_accounts_without_a_factor() {
        let instance = settings(false, false);
        // A password and nothing else: closed until there is a factor.
        assert!(workspace_blocked(&instance, true, NONE));
        assert!(!workspace_blocked(&instance, true, APP));
        assert!(!workspace_blocked(&instance, true, PASSKEY));
        // Google-only accounts, as for the instance's own requirement: only
        // once Google sign-ins ask for codes.
        assert!(!workspace_blocked(&instance, false, NONE));
        assert!(workspace_blocked(&settings(false, true), false, NONE));
    }

    #[test]
    fn every_workspace_requires_it_while_the_instance_does() {
        // Its admins choose, until the instance requires it.
        assert!(!workspace_requires(&settings(false, false), false));
        assert!(workspace_requires(&settings(false, false), true));
        assert!(!workspace_requires(&settings(false, true), false));
        assert!(workspace_requires(&settings(true, false), false));
        assert!(workspace_requires(&settings(true, true), false));
        // Then it applies to exactly the accounts the instance's requirement
        // closes until they have a second factor: no workspace needs to
        // close to them on its own.
        for google in [false, true] {
            let instance = settings(true, google);
            for has_password in [false, true] {
                for factors in [NONE, APP, PASSKEY] {
                    assert_eq!(
                        workspace_blocked(&instance, has_password, factors),
                        setup_required(&instance, has_password, factors),
                        "google: {google}, password: {has_password}, {factors:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_workspace_requirement_applies_like_the_instances() {
        let instance = settings(false, false);
        let member = with_workspaces(&instance, true);
        // A passkey becomes the second factor of password sign-ins, and the
        // last factor can't go.
        assert!(on(&member, true, PASSKEY));
        assert!(!on(&instance, true, PASSKEY));
        assert!(!can_turn_off(&member, true, APP));
        assert!(!can_remove_last_passkey(&member, true, PASSKEY));
        assert!(asks(&member, Via::Password, true, PASSKEY, false));
        // Nothing changes for people outside such workspaces.
        assert_eq!(with_workspaces(&instance, false), instance);
    }

    #[test]
    fn requiring_it_covers_password_sign_ins_and_google_when_it_asks() {
        // Not required: nobody needs it.
        assert!(!required(&settings(false, false), true));
        assert!(!required(&settings(false, true), true));
        // Required: accounts with a password do; Google-only ones only when
        // Google sign-ins ask for codes too.
        assert!(required(&settings(true, false), true));
        assert!(!required(&settings(true, false), false));
        assert!(required(&settings(true, true), false));
        assert!(setup_required(&settings(true, false), true, NONE));
        assert!(!setup_required(&settings(true, false), true, APP));
        assert!(!setup_required(&settings(true, false), true, PASSKEY));
        assert!(!setup_required(&settings(true, false), false, NONE));
        assert!(!can_turn_off(&settings(true, false), true, APP));
        assert!(can_turn_off(
            &settings(true, false),
            true,
            Factors {
                passkeys: true,
                ..APP
            }
        ));
        assert!(can_turn_off(&settings(true, false), false, APP));
        assert!(can_turn_off(&settings(false, true), true, APP));
        assert!(!can_remove_last_passkey(
            &settings(true, false),
            true,
            PASSKEY
        ));
        assert!(can_remove_last_passkey(
            &settings(true, false),
            true,
            Factors {
                authenticator: true,
                ..PASSKEY
            }
        ));
        assert!(can_remove_last_passkey(
            &settings(false, false),
            true,
            PASSKEY
        ));
    }

    #[test]
    fn passwords_always_ask_and_google_when_chosen() {
        let off = settings(false, false);
        assert!(asks(&off, Via::Password, true, APP, false));
        assert!(!asks(&off, Via::Password, true, NONE, false));
        assert!(!asks(&off, Via::Google, true, APP, false));
        assert!(asks(&off, Via::Google, true, APP, true));
        // The instance asks it of everyone with a second factor.
        let google = settings(false, true);
        assert!(asks(&google, Via::Google, true, APP, false));
        assert!(!asks(&google, Via::Google, true, NONE, false));
    }

    #[test]
    fn passkeys_are_a_second_factor_only_where_it_is_required() {
        // A passkey alone doesn't make password sign-ins ask for anything…
        let off = settings(false, false);
        assert!(!on(&off, true, PASSKEY));
        assert!(!asks(&off, Via::Password, true, PASSKEY, true));
        // …until the instance requires two-factor: then it is the factor.
        let required = settings(true, false);
        assert!(on(&required, true, PASSKEY));
        assert!(asks(&required, Via::Password, true, PASSKEY, false));
        assert!(!asks(&required, Via::Google, true, PASSKEY, false));
        // Google-only accounts, once Google sign-ins ask too.
        let everyone = settings(true, true);
        assert!(asks(&everyone, Via::Google, false, PASSKEY, false));
    }

    #[test]
    fn wrong_codes_cost_more_each_time() {
        for f in 0..FREE_FAILURES {
            assert_eq!(lockout_ms(f), None);
        }
        assert_eq!(lockout_ms(5), Some(60_000));
        assert_eq!(lockout_ms(6), Some(120_000));
        assert_eq!(lockout_ms(7), Some(240_000));
        assert_eq!(lockout_ms(10), Some(1_920_000));
        assert_eq!(lockout_ms(11), Some(3_600_000));
        assert_eq!(lockout_ms(500), Some(3_600_000));
        assert_eq!(lockout_ms(u32::MAX), Some(3_600_000));
    }

    #[test]
    fn recovery_codes_are_readable_and_forgiving() {
        let codes = new_recovery_codes();
        assert_eq!(codes.len(), RECOVERY_CODES);
        for code in &codes {
            assert_eq!(code.len(), 11, "{code}");
            assert_eq!(code.as_bytes()[5], b'-');
            let plain = normalize_recovery_code(code).unwrap_or_default();
            assert_eq!(plain, code.replace('-', ""));
            assert_eq!(
                normalize_recovery_code(&format!(" {} ", code.to_lowercase().replace('-', " "))),
                Some(plain.clone())
            );
            assert_eq!(hash_recovery_code(code), Some(hash_secret(&plain)));
        }
        let unique: std::collections::HashSet<&String> = codes.iter().collect();
        assert_eq!(unique.len(), codes.len());
        // Vowels, look-alikes and wrong lengths can't be recovery codes.
        assert_eq!(normalize_recovery_code("AAAAA-BBBBB"), None);
        assert_eq!(normalize_recovery_code("BCDFG-HJKM0"), None);
        assert_eq!(normalize_recovery_code("BCDFG-HJKM"), None);
        assert_eq!(normalize_recovery_code("123456"), None);
        assert_eq!(hash_recovery_code(""), None);
    }
}
