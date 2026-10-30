//! Confirming it's you before sensitive actions
//! (docs/architecture.md#confirming-its-you).
//!
//! Where the instance asks for it (`verify_sensitive_actions`, which only
//! the primary instance owner turns on or off), the actions in
//! [`Sensitive`] take the person's passkey or a code from their
//! authenticator app first, when they have one: a stolen browser session,
//! CLI or access token can't do them alone. A confirmation belongs to the
//! browser, CLI or token that made it, for [`VERIFIED_MS`]; a sign-in with
//! a passkey, or with a second factor, counts as one. People with neither a
//! passkey nor an authenticator app have nothing to confirm it with, and
//! aren't asked.

use crate::two_factor::Factors;
use fctl_proto::ApiError;
use fctl_proto::api::{InstanceSettings, SecondFactor, UpdateInstanceSettingsRequest};
use fctl_proto::session::{InstanceRole, WorkspaceRole};

/// How long a confirmation lasts, for the browser, CLI or token that made
/// it.
pub const VERIFIED_MS: i64 = 15 * 60 * 1000;

/// An action that takes confirming it's you, where the instance asks for
/// that.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sensitive {
    /// Inviting someone to the instance as an admin, or letting an address
    /// sign up as one (the allowlist).
    InviteAdmin,
    /// Making someone an instance admin.
    MakeAdmin,
    /// Handing the primary instance owner's role to an admin.
    TransferInstance,
    /// Deleting someone else's account (instance admins).
    DeleteUser,
    /// Making a password reset link for someone that comes back to the
    /// admin, where the instance sends no email (instance admins): whoever
    /// has it signs in as them.
    ResetLink,
    /// Resetting someone's two-factor sign-in (instance admins).
    ResetTwoFactor,
    /// No longer requiring two-factor sign-in of everyone.
    StopRequiringTwoFactor,
    /// No longer asking everyone for codes on Google sign-in.
    StopGoogleCodes,
    /// No longer asking people to confirm sensitive actions.
    StopVerifying,
    /// No longer requiring two-factor sign-in in a workspace.
    StopWorkspaceTwoFactor,
    /// Adding a runner someone registered to a workspace, or letting it
    /// run everyone's sessions there.
    AddRunner,
    /// Making someone a workspace's admin, or inviting them as one.
    MakeWorkspaceAdmin,
    /// Handing a workspace's primary admin role over, or giving it to one
    /// of its members (instance admins).
    TransferWorkspace,
    DeleteWorkspace,
    DeleteProject,
    /// Making an access token.
    NewToken,
    /// Connecting a Claude subscription, with a token, a sign-in or the
    /// token kept on their runners: the sessions the person starts then run
    /// on it, and a full login's Claude account sees them (Remote Control),
    /// so a stolen session or token could otherwise have their sessions run
    /// where it watches, for good.
    ConnectClaude,
    /// Linking a Linear account: what it delegates to fctl or mentions it
    /// in then starts sessions as the person, on their subscription, and
    /// its replies reach their sessions' agents, so a stolen session or
    /// token could otherwise go on starting their sessions from Linear long
    /// after it was revoked.
    LinkLinear,
    /// Approving a CLI's sign-in, or confirming it's you for a CLI that is
    /// signed in already.
    ApproveCli,
    /// Removing a passkey: the last one may be the account's second factor.
    RemovePasskey,
    /// Exporting your data.
    Export,
    /// Deleting your own account.
    DeleteAccount,
}

impl Sensitive {
    /// What it does, after "to": "Confirm it's you to delete a project".
    pub fn doing(self) -> &'static str {
        use Sensitive::*;
        match self {
            InviteAdmin => "invite an admin",
            MakeAdmin => "make someone an admin",
            TransferInstance => "hand over the primary instance owner's role",
            DeleteUser => "delete someone's account",
            ResetLink => "make a password reset link for someone",
            ResetTwoFactor => "reset someone's two-factor sign-in",
            StopRequiringTwoFactor => "stop requiring two-factor sign-in",
            StopGoogleCodes => "stop asking for codes on Google sign-in",
            StopVerifying => "stop asking people to confirm sensitive actions",
            StopWorkspaceTwoFactor => "stop requiring two-factor sign-in in a workspace",
            AddRunner => "add a runner to a workspace",
            MakeWorkspaceAdmin => "make someone a workspace admin",
            TransferWorkspace => "change a workspace's primary admin",
            DeleteWorkspace => "delete a workspace",
            DeleteProject => "delete a project",
            NewToken => "make an access token",
            ConnectClaude => "connect a Claude subscription",
            LinkLinear => "link a Linear account",
            ApproveCli => "approve the CLI",
            RemovePasskey => "remove a passkey",
            Export => "export your data",
            DeleteAccount => "delete your account",
        }
    }
}

/// Whether sensitive actions ask this person to confirm it's them: the
/// instance asks for it, and they have a passkey or an authenticator app to
/// confirm it with.
pub fn required(settings: &InstanceSettings, factors: Factors) -> bool {
    settings.verify_sensitive_actions && factors.any()
}

/// Until when a confirmation made at `verified_at` counts, if it still does
/// at `now`.
pub fn until(verified_at: Option<i64>, now: i64) -> Option<i64> {
    verified_at
        .map(|at| at.saturating_add(VERIFIED_MS))
        .filter(|until| *until > now)
}

/// Whether a sensitive action waits for the person to confirm it's them:
/// it is [`required`] of them, and the browser, CLI or token they use
/// hasn't confirmed within the last [`VERIFIED_MS`].
pub fn needed(
    settings: &InstanceSettings,
    factors: Factors,
    verified_at: Option<i64>,
    now: i64,
) -> bool {
    required(settings, factors) && until(verified_at, now).is_none()
}

/// What they can confirm it's them with: a passkey, a code from the
/// authenticator app, or one of its recovery codes, as a sign-in's second
/// step takes them.
pub fn methods(factors: Factors, recovery_codes: u32) -> Vec<SecondFactor> {
    let mut methods = Vec::new();
    if factors.passkeys {
        methods.push(SecondFactor::Passkey);
    }
    if factors.authenticator {
        methods.push(SecondFactor::Totp);
        if recovery_codes > 0 {
            methods.push(SecondFactor::RecoveryCode);
        }
    }
    methods
}

/// What a request that waits for it answers: what to confirm, and with
/// what.
pub fn message(action: Sensitive, factors: Factors) -> String {
    let with = match (factors.passkeys, factors.authenticator) {
        (true, true) => "your passkey or a code from your authenticator app",
        (true, false) => "your passkey",
        _ => "a code from your authenticator app",
    };
    format!("Confirm it's you to {}, with {with}.", action.doing())
}

/// The sensitive action a change to the instance's settings is, if any:
/// turning off what makes people prove it's them. Turning things on is
/// never one.
pub fn instance_settings_change(
    current: &InstanceSettings,
    update: &UpdateInstanceSettingsRequest,
) -> Option<Sensitive> {
    let off = |now: bool, next: Option<bool>| now && next == Some(false);
    if off(
        current.verify_sensitive_actions,
        update.verify_sensitive_actions,
    ) {
        Some(Sensitive::StopVerifying)
    } else if off(current.require_two_factor, update.require_two_factor) {
        Some(Sensitive::StopRequiringTwoFactor)
    } else if off(current.two_factor_google, update.two_factor_google) {
        Some(Sensitive::StopGoogleCodes)
    } else {
        None
    }
}

/// Whether someone whose instance role is `actor` may turn confirming
/// sensitive actions on or off (from `current` to `next`): only the
/// primary instance owner.
pub fn check_verify_setting(
    actor: InstanceRole,
    current: bool,
    next: bool,
) -> Result<(), ApiError> {
    if current != next && actor != InstanceRole::Owner {
        return Err(ApiError::forbidden(
            "Only the primary instance owner turns confirming sensitive actions on or off",
        ));
    }
    Ok(())
}

/// Whether giving someone the instance role `to` (from `from`) makes them
/// an admin.
pub fn makes_admin(from: InstanceRole, to: InstanceRole) -> bool {
    to == InstanceRole::Admin && from == InstanceRole::Member
}

/// Whether giving someone the workspace role `to` (from `from`, `None` for
/// someone invited) makes them one of its admins.
pub fn makes_workspace_admin(from: Option<WorkspaceRole>, to: WorkspaceRole) -> bool {
    to.is_admin() && !from.is_some_and(WorkspaceRole::is_admin)
}

/// Whether adding a runner to a workspace widens what it runs there: it
/// isn't in the workspace yet (`now` is `None`), or it ran only its
/// owner's sessions there (`Some(true)`) and now runs everyone's.
pub fn adds_runner(now: Option<bool>, personal: bool) -> bool {
    match now {
        None => true,
        Some(was_personal) => was_personal && !personal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Factors = Factors {
        authenticator: false,
        passkeys: false,
    };
    const APP: Factors = Factors {
        authenticator: true,
        passkeys: false,
    };
    const PASSKEY: Factors = Factors {
        authenticator: false,
        passkeys: true,
    };
    const BOTH: Factors = Factors {
        authenticator: true,
        passkeys: true,
    };

    fn asking(on: bool) -> InstanceSettings {
        InstanceSettings {
            verify_sensitive_actions: on,
            ..Default::default()
        }
    }

    #[test]
    fn only_people_with_a_passkey_or_an_app_are_asked_and_only_where_the_instance_asks() {
        // Off by default: nobody is asked.
        assert!(!InstanceSettings::default().verify_sensitive_actions);
        for factors in [NONE, APP, PASSKEY, BOTH] {
            assert!(!required(&asking(false), factors));
            assert!(!needed(&asking(false), factors, None, 0));
        }
        // On: whoever has something to confirm it with.
        assert!(!required(&asking(true), NONE));
        assert!(required(&asking(true), APP));
        assert!(required(&asking(true), PASSKEY));
        assert!(required(&asking(true), BOTH));
    }

    #[test]
    fn a_confirmation_lasts_fifteen_minutes() {
        let at = 1_000_000;
        assert!(needed(&asking(true), PASSKEY, None, at));
        assert!(!needed(&asking(true), PASSKEY, Some(at), at));
        assert!(!needed(
            &asking(true),
            PASSKEY,
            Some(at),
            at + VERIFIED_MS - 1
        ));
        assert!(needed(&asking(true), PASSKEY, Some(at), at + VERIFIED_MS));
        assert_eq!(until(Some(at), at + 1), Some(at + VERIFIED_MS));
        assert_eq!(until(Some(at), at + VERIFIED_MS), None);
        assert_eq!(until(None, at), None);
        // A time from the far future can't overflow into the past.
        assert_eq!(until(Some(i64::MAX), at), Some(i64::MAX));
    }

    #[test]
    fn people_confirm_with_what_they_have() {
        assert!(methods(NONE, 0).is_empty());
        assert_eq!(methods(PASSKEY, 0), vec![SecondFactor::Passkey]);
        assert_eq!(methods(APP, 0), vec![SecondFactor::Totp]);
        assert_eq!(
            methods(BOTH, 3),
            vec![
                SecondFactor::Passkey,
                SecondFactor::Totp,
                SecondFactor::RecoveryCode
            ]
        );
        // Recovery codes stand in for the app only.
        assert_eq!(methods(PASSKEY, 3), vec![SecondFactor::Passkey]);
    }

    #[test]
    fn the_message_says_what_and_with_what() {
        assert_eq!(
            message(Sensitive::DeleteProject, PASSKEY),
            "Confirm it's you to delete a project, with your passkey."
        );
        assert_eq!(
            message(Sensitive::Export, APP),
            "Confirm it's you to export your data, with a code from your authenticator app."
        );
        assert_eq!(
            message(Sensitive::NewToken, BOTH),
            "Confirm it's you to make an access token, with your passkey or a code from your authenticator app."
        );
        assert_eq!(
            message(Sensitive::ConnectClaude, APP),
            "Confirm it's you to connect a Claude subscription, with a code from your authenticator app."
        );
        assert_eq!(
            message(Sensitive::LinkLinear, APP),
            "Confirm it's you to link a Linear account, with a code from your authenticator app."
        );
    }

    #[test]
    fn turning_protections_off_is_sensitive_and_turning_them_on_isnt() {
        let all_on = InstanceSettings {
            require_two_factor: true,
            two_factor_google: true,
            verify_sensitive_actions: true,
            ..Default::default()
        };
        let update = |verify, require, google| UpdateInstanceSettingsRequest {
            verify_sensitive_actions: verify,
            require_two_factor: require,
            two_factor_google: google,
            ..Default::default()
        };
        assert_eq!(
            instance_settings_change(&all_on, &update(Some(false), None, None)),
            Some(Sensitive::StopVerifying)
        );
        assert_eq!(
            instance_settings_change(&all_on, &update(None, Some(false), None)),
            Some(Sensitive::StopRequiringTwoFactor)
        );
        assert_eq!(
            instance_settings_change(&all_on, &update(None, None, Some(false))),
            Some(Sensitive::StopGoogleCodes)
        );
        // Keeping them on, or saying nothing about them.
        assert_eq!(
            instance_settings_change(&all_on, &update(Some(true), Some(true), Some(true))),
            None
        );
        assert_eq!(
            instance_settings_change(&all_on, &UpdateInstanceSettingsRequest::default()),
            None
        );
        // Off already, or turning them on.
        let all_off = InstanceSettings::default();
        assert_eq!(
            instance_settings_change(&all_off, &update(Some(false), Some(false), Some(false))),
            None
        );
        assert_eq!(
            instance_settings_change(&all_off, &update(Some(true), Some(true), Some(true))),
            None
        );
        // Publishing screenshots isn't about proving it's you.
        let screenshots = UpdateInstanceSettingsRequest {
            publish_screenshots: Some(false),
            ..Default::default()
        };
        let publishing = InstanceSettings {
            publish_screenshots: true,
            ..all_on
        };
        assert_eq!(instance_settings_change(&publishing, &screenshots), None);
    }

    #[test]
    fn only_the_primary_instance_owner_turns_it_on_or_off() {
        for (current, next) in [(false, true), (true, false)] {
            assert!(check_verify_setting(InstanceRole::Owner, current, next).is_ok());
            assert!(check_verify_setting(InstanceRole::Admin, current, next).is_err());
            assert!(check_verify_setting(InstanceRole::Member, current, next).is_err());
        }
        // Saying what it is already changes nothing, whoever says it.
        assert!(check_verify_setting(InstanceRole::Admin, true, true).is_ok());
        assert!(check_verify_setting(InstanceRole::Admin, false, false).is_ok());
    }

    #[test]
    fn promotions_are_sensitive_and_demotions_arent() {
        use InstanceRole as I;
        use WorkspaceRole as W;
        assert!(makes_admin(I::Member, I::Admin));
        assert!(!makes_admin(I::Admin, I::Admin));
        assert!(!makes_admin(I::Admin, I::Member));
        assert!(makes_workspace_admin(None, W::Admin));
        assert!(makes_workspace_admin(Some(W::Member), W::Admin));
        assert!(!makes_workspace_admin(Some(W::Admin), W::Admin));
        assert!(!makes_workspace_admin(Some(W::Owner), W::Admin));
        assert!(!makes_workspace_admin(Some(W::Admin), W::Member));
        assert!(!makes_workspace_admin(None, W::Member));
    }

    #[test]
    fn adding_a_runner_is_sensitive_when_it_runs_more_there() {
        // New to the workspace, for whoever's sessions.
        assert!(adds_runner(None, true));
        assert!(adds_runner(None, false));
        // From its owner's sessions to everyone's.
        assert!(adds_runner(Some(true), false));
        // The same, or fewer.
        assert!(!adds_runner(Some(true), true));
        assert!(!adds_runner(Some(false), false));
        assert!(!adds_runner(Some(false), true));
    }
}
