//! REST request and response bodies for `/api/v1`.
//!
//! Conventions: JSON in and out; errors are [`crate::ApiError`] with an HTTP
//! status from [`crate::ErrorCode::http_status`]; times are [`crate::Millis`];
//! list endpoints return objects (`{"sessions": [...]}`) so they can grow.

use crate::event::SessionEvent;
use crate::runner::{ClaudeCredentialKind, DriverInfo, SandboxLimits};
use crate::session::{
    GithubAccess, InstanceRole, Memory, Project, ProjectRole, ProjectSettings, RepoAccess, RepoRef,
    Retrospective, RetrospectiveOutcome, Session, SessionRef, SessionRole, SessionStatus, UserRef,
    WorkspaceColor, WorkspaceRef, WorkspaceRole,
};
use crate::{
    AccessRequestId, AllowId, AttachmentId, CredentialId, DeliveryId, IdentityId, InviteId,
    LoginJobId, MemoryId, Millis, PasskeyId, ProjectId, PushDeviceId, RetrospectiveId, RunnerId,
    ScheduleId, ScheduleRunId, SessionId, TokenId, UserId, WorkspaceId, WorkspaceInviteId,
};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

// ───────────────────────────── instance ─────────────────────────────

/// `GET /api/v1/instance` (public): what the sign-in page needs to know.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InstanceInfo {
    pub name: String,
    pub version: String,
    /// No owner yet: the setup link must be used first.
    pub setup_required: bool,
    pub features: Features,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Features {
    /// "Continue with Google" is configured.
    pub google: bool,
    /// Transactional email is configured (resets and invites are emailed).
    pub email: bool,
    /// A GitHub App is configured.
    pub github: bool,
    /// The GitHub App's webhook is on (`GITHUB_WEBHOOK_SECRET`): GitHub
    /// keeps sessions' pull requests current, and what happens to them can
    /// start review agents and bring agents back to them
    /// ([`crate::session::FollowUps`]). Absent from control planes before
    /// it.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub github_webhook: bool,
    /// Admins let projects publish screenshots ([`InstanceSettings`]).
    #[serde(default)]
    pub publish_screenshots: bool,
    /// People without an account ask for early access from the sign-in
    /// page ([`RequestAccessRequest`]), and admins invite them from the
    /// list ([`AccessRequestList`]): where the deployment turns it on
    /// (`FCTL_EARLY_ACCESS`), as app.fctl.sh's does.
    #[serde(default)]
    pub early_access: bool,
}

/// `POST /api/v1/setup`: create the owner account with the setup token.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SetupRequest {
    pub token: String,
    pub email: String,
    /// First and last name; or `name`, the full name (older clients).
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub name: Option<String>,
    pub password: String,
}

// ───────────────────────────── users and auth ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum UserStatus {
    Active,
    Disabled,
}

impl UserStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct User {
    pub id: UserId,
    pub email: String,
    /// The full name everyone sees.
    pub name: String,
    /// First names, all of them ("Carl Oscar"); what greetings use.
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub avatar_url: Option<String>,
    pub role: InstanceRole,
    pub status: UserStatus,
    /// Whether the account can sign in with a password.
    pub has_password: bool,
    /// Whether the account has two-factor sign-in: an authenticator app.
    #[serde(default)]
    pub two_factor: bool,
    /// How many passkeys the account signs in with.
    #[serde(default)]
    pub passkeys: u32,
    pub created_at: Millis,
    pub last_login_at: Option<Millis>,
    /// Its other email addresses, which also sign in to it (for instance
    /// admins; empty elsewhere).
    #[serde(default)]
    pub emails: Vec<String>,
    /// How many workspaces it is a member of (for instance admins).
    #[serde(default)]
    pub workspaces: u32,
}

impl User {
    pub fn to_ref(&self) -> UserRef {
        UserRef {
            id: self.id.clone(),
            name: self.name.clone(),
            avatar_url: self.avatar_url.clone(),
        }
    }
}

/// A linked external account.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Identity {
    pub id: IdentityId,
    /// `google`.
    pub provider: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub created_at: Millis,
}

/// `GET /api/v1/auth/me`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Me {
    pub user: User,
    pub identities: Vec<Identity>,
    pub claude: Vec<ClaudeCredentialInfo>,
    /// The address the commits of sessions you start credit you with, in a
    /// `Co-authored-by` trailer. Only you see it here; absent when you have
    /// none, and then commits don't name you.
    pub commit_email: Option<String>,
    /// Two-factor sign-in: what you have set up, and what the instance
    /// requires.
    #[serde(default)]
    pub two_factor: TwoFactor,
    /// The passkeys you sign in with.
    #[serde(default)]
    pub passkeys: Vec<Passkey>,
    /// Your email addresses, the primary one first. You sign in with any of
    /// them, and each workspace knows you by one.
    #[serde(default)]
    pub emails: Vec<EmailAddress>,
    /// The access token of the request, when it is one. A limited one sees
    /// only `user` and this here: the rest stays empty.
    pub token: Option<TokenInfo>,
}

/// One of your email addresses. Each was invited to the instance on its
/// own; accepting an invitation while signed in connects it to your account.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct EmailAddress {
    pub email: String,
    /// The account's address: where fctl writes to you (password resets,
    /// sign-in alerts), and what instance admins see first.
    pub primary: bool,
    /// The address the commits of sessions you start in the workspaces you
    /// are a member as this address credit you with (`Co-authored-by`).
    /// Absent: those commits don't name you.
    pub commit_email: Option<String>,
    /// The workspaces you are a member as this address: the ones that see it.
    pub workspaces: Vec<WorkspaceRef>,
    pub added_at: Millis,
}

/// `PATCH /api/v1/me/emails/{email}`: absent fields stay as they are.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateEmailRequest {
    /// The commit email for this address; an empty one removes it.
    pub commit_email: Option<String>,
    /// `true` makes it the account's primary address.
    pub primary: Option<bool>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SignupRequest {
    pub email: String,
    /// First and last name; or `name`, the full name (older clients).
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub name: Option<String>,
    pub password: String,
    /// The token from an invitation link.
    pub invite: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// Returned by setup, signup, login, password reset and the second factor
/// (`POST /auth/two-factor/verify`). The web session is set as a cookie.
/// When the account has two-factor sign-in, login and password reset
/// answer with `two_factor` instead of `me`: the sign-in waits for a code,
/// and its challenge is set as a cookie.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AuthResponse {
    /// Who signed in; absent while the sign-in waits for its second factor.
    pub me: Option<Me>,
    /// The sign-in waits for a code: who it is for and what finishes it.
    pub two_factor: Option<TwoFactorChallenge>,
    /// Where to go next: the page a Google sign-in started from.
    pub next: Option<String>,
    /// Signed in with a recovery code: how many are left.
    pub recovery_codes_left: Option<u32>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateMeRequest {
    /// First and last name (send both; an empty last name clears it).
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    /// The full name alone, from older clients: the parts become unknown.
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    /// The address commits credit you with (`Me.commit_email`); an empty
    /// one removes it.
    pub commit_email: Option<String>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ChangePasswordRequest {
    /// Required when the account already has a password.
    pub current_password: Option<String>,
    pub new_password: String,
}

/// `GET /api/v1/me/deletion`: what deleting your account would delete.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AccountDeletion {
    /// Why the account cannot be deleted now; absent when it can.
    pub blocked: Option<String>,
    /// The projects of the workspaces only you are a member of: deleted,
    /// with their sessions.
    pub projects: Vec<DeletedProject>,
    /// Sessions you started in workspaces others are members of: they stay
    /// with their projects, attributed to a deleted user.
    pub kept_sessions: u32,
    /// Workspaces only you are a member of: deleted.
    #[serde(default)]
    pub workspaces: Vec<WorkspaceRef>,
    /// Workspaces you are the primary admin of, with others in them: the
    /// role passes to their longest-standing admin, or member.
    #[serde(default)]
    pub handovers: Vec<Handover>,
}

/// Who becomes a workspace's primary admin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Handover {
    pub workspace: WorkspaceRef,
    pub to: UserRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeletedProject {
    pub slug: String,
    pub name: String,
    pub sessions: u32,
}

/// `POST /api/v1/me/delete`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeleteAccountRequest {
    /// The account's email address, typed to confirm.
    pub email: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ForgotPasswordRequest {
    pub email: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResetPasswordRequest {
    pub token: String,
    pub new_password: String,
}

// ───────────────────────────── two-factor sign-in ─────────────────────────────

/// Two-factor sign-in, as `Me.two_factor` reports it: a code from an
/// authenticator app after the password, and after Google when the person
/// or the instance asks for that. Recovery codes stand in for the app.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(default)]
pub struct TwoFactor {
    /// When the authenticator app was set up; absent without one.
    pub authenticator_since: Option<Millis>,
    /// When a code from it last signed you in.
    pub authenticator_used_at: Option<Millis>,
    /// Recovery codes not used yet.
    pub recovery_codes: u32,
    /// Google sign-ins ask for a code too, because you chose that.
    pub google: bool,
    /// Google sign-ins ask everyone with two-factor for a code
    /// ([`InstanceSettings::two_factor_google`]).
    pub google_required: bool,
    /// The instance requires two-factor of this account: it signs in with
    /// a password, or with Google once Google sign-ins ask for codes.
    pub required: bool,
    /// Required, and neither an authenticator app nor a passkey is set up:
    /// until one is, the account can only set one up (requests fail with
    /// `two_factor_setup_required`).
    pub setup_required: bool,
    /// Sign-ins with a password ask for a second factor: there is an
    /// authenticator app, or the instance requires one and the account's
    /// passkeys are it.
    pub on: bool,
}

/// What can finish a sign-in that waits for its second factor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SecondFactor {
    /// A code from the authenticator app.
    Totp,
    /// One of the account's recovery codes.
    RecoveryCode,
    /// One of the account's passkeys.
    Passkey,
}

/// Why a sign-in waits for its second factor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ChallengeReason {
    /// The password was right.
    Password,
    /// Google signed the person in.
    Google,
    /// A reset link chose a new password, which takes effect with the code.
    PasswordReset,
}

impl ChallengeReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Google => "google",
            Self::PasswordReset => "password_reset",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Self::Password, Self::Google, Self::PasswordReset]
            .into_iter()
            .find(|r| r.as_str() == s)
    }
}

/// A sign-in that waits for its second factor (`AuthResponse.two_factor`,
/// `GET /api/v1/auth/two-factor/challenge`). The browser holds it in the
/// `__Host-fctl-2fa` cookie for ten minutes.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct TwoFactorChallenge {
    /// Who is signing in.
    pub name: String,
    pub email: String,
    pub avatar_url: Option<String>,
    pub reason: ChallengeReason,
    /// What can finish it.
    pub methods: Vec<SecondFactor>,
    pub expires_at: Millis,
    /// After too many wrong codes: codes are taken again from then on.
    pub locked_until: Option<Millis>,
}

/// `POST /api/v1/auth/two-factor/verify`: finishes the waiting sign-in, with
/// a code or a passkey.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(default)]
pub struct VerifyTwoFactorRequest {
    /// A code from the authenticator app (six digits) or a recovery code.
    pub code: Option<String>,
    /// A passkey's answer to `POST /api/v1/auth/two-factor/passkey`'s
    /// options.
    pub passkey: Option<PasskeyAssertion>,
}

/// `GET /api/v1/auth/verify`: whether sensitive actions ask you to confirm
/// it's you first, and what you can confirm it with.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(default)]
pub struct Verification {
    /// The instance asks for it
    /// ([`InstanceSettings::verify_sensitive_actions`]), and you have a
    /// passkey or an authenticator app to confirm it's you with.
    pub required: bool,
    /// The browser, CLI or token that asks counts as confirmed now, by the
    /// server's clock: sensitive actions don't ask it until
    /// `verified_until`.
    pub verified: bool,
    /// Until when the browser, CLI or token that asks counts as confirmed:
    /// 15 minutes after it last confirmed it's you, or signed in with a
    /// passkey or a second factor. Absent when it doesn't.
    pub verified_until: Option<Millis>,
    /// What you can confirm it's you with.
    pub methods: Vec<SecondFactor>,
    /// After too many wrong codes: codes from the app are taken again from
    /// then on.
    pub locked_until: Option<Millis>,
}

/// `POST /api/v1/auth/verify`: confirms it's you before sensitive actions,
/// with a code or a passkey. The answer is the [`Verification`] after it.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(default)]
pub struct VerifyRequest {
    /// A code from the authenticator app (six digits) or a recovery code.
    pub code: Option<String>,
    /// A passkey's answer to `POST /api/v1/auth/verify/passkey`'s options.
    pub passkey: Option<PasskeyAssertion>,
}

/// `POST /api/v1/me/two-factor/authenticator`: starts setting up an
/// authenticator app.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(default)]
pub struct StartAuthenticatorRequest {
    /// Replacing the app you have takes a code from it, or a recovery code.
    pub code: Option<String>,
}

/// A new secret for an authenticator app. Nothing changes until a code from
/// the app confirms it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AuthenticatorSetup {
    /// The secret in base32, for apps that can't scan.
    pub secret: String,
    /// The `otpauth://totp/…` link, which the QR code holds and phones open
    /// in their authenticator app.
    pub uri: String,
    /// The link as a QR code: one string per row, `1` for a dark module
    /// and `0` for a light one, without the quiet zone around it.
    pub qr: Vec<String>,
    /// Sent back with the first code; holds the secret, sealed for you.
    pub setup: String,
    pub expires_at: Millis,
    /// Confirming takes your password: this browser signed in more than 15
    /// minutes ago, or the request came with a CLI or access token.
    pub password_required: bool,
}

/// `POST /api/v1/me/two-factor/authenticator/confirm`: turns the new
/// authenticator app on, in place of the old one if there was one.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ConfirmAuthenticatorRequest {
    /// `AuthenticatorSetup.setup`.
    pub setup: String,
    /// The code the app shows now.
    pub code: String,
    /// When `AuthenticatorSetup.password_required`.
    pub password: Option<String>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AuthenticatorAdded {
    pub me: Me,
    /// New recovery codes, shown once: when there were none. Replacing the
    /// app keeps the ones you have.
    pub recovery_codes: Option<Vec<String>>,
}

/// Turning two-factor off (`POST /api/v1/me/two-factor/authenticator/remove`)
/// and new recovery codes (`POST /api/v1/me/two-factor/recovery-codes`)
/// take a current code.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TwoFactorCodeRequest {
    /// A code from the authenticator app, or a recovery code.
    pub code: String,
}

/// New recovery codes, shown once; the old ones stop working.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RecoveryCodes {
    pub codes: Vec<String>,
    pub me: Me,
}

/// `PATCH /api/v1/me/two-factor`.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateTwoFactorRequest {
    /// Google sign-ins ask for a code too.
    pub google: Option<bool>,
}

// ───────────────────────────── passkeys ─────────────────────────────

/// A passkey: signs its person in with their fingerprint, face or screen
/// lock, without a password or code. Where two-factor is required, it is
/// also a second factor.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Passkey {
    pub id: PasskeyId,
    pub name: String,
    /// Who keeps it, by the authenticator's model, when that is known:
    /// "1Password", "Apple Passwords", "Windows Hello".
    pub provider: Option<String>,
    /// It syncs between its person's devices through its provider.
    pub synced: bool,
    /// The WebAuthn credential id (base64url), for the browser's signals
    /// that keep providers' lists current.
    pub credential_id: String,
    pub created_at: Millis,
    pub last_used_at: Option<Millis>,
}

/// How a passkey's options name a credential (`PublicKeyCredentialDescriptor`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct PasskeyDescriptor {
    /// `public-key`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The credential id, base64url.
    pub id: String,
    pub transports: Option<Vec<String>>,
}

/// The site a passkey belongs to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PasskeyRp {
    /// The app's host name.
    pub id: String,
    pub name: String,
}

/// Whom a passkey signs in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PasskeyUser {
    /// The user handle, base64url: the account's id, never their email.
    pub id: String,
    /// The email address, which passkey providers list it by.
    pub name: String,
    pub display_name: String,
}

/// An algorithm a new passkey may use (`PublicKeyCredentialParameters`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PasskeyAlgorithm {
    #[serde(rename = "type")]
    pub kind: String,
    /// COSE: -7 ES256, -8 EdDSA, -257 RS256.
    pub alg: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PasskeyAuthenticatorSelection {
    /// `required`: a passkey, which the authenticator finds by itself.
    pub resident_key: String,
    pub require_resident_key: bool,
    /// `required`: the device checks who it is.
    pub user_verification: String,
}

/// `POST /api/v1/me/passkeys/options`: what `navigator.credentials.create`
/// takes to make a passkey, in WebAuthn's JSON form
/// (`PublicKeyCredential.parseCreationOptionsFromJSON`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PasskeyCreationOptions {
    pub rp: PasskeyRp,
    pub user: PasskeyUser,
    /// base64url; works once, for ten minutes.
    pub challenge: String,
    pub pub_key_cred_params: Vec<PasskeyAlgorithm>,
    /// Milliseconds.
    pub timeout: u32,
    /// The passkeys the account has, so that an authenticator doesn't make
    /// a second.
    pub exclude_credentials: Vec<PasskeyDescriptor>,
    pub authenticator_selection: PasskeyAuthenticatorSelection,
    /// `none`: fctl asks nobody which authenticator this is.
    pub attestation: String,
}

/// What `navigator.credentials.get` takes to sign in with a passkey, in
/// WebAuthn's JSON form (`PublicKeyCredential.parseRequestOptionsFromJSON`):
/// `POST /api/v1/auth/passkey/options`, and as a second factor
/// `POST /api/v1/auth/two-factor/passkey`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PasskeyRequestOptions {
    /// base64url; works once, for ten minutes.
    pub challenge: String,
    pub rp_id: String,
    pub timeout: u32,
    /// Empty when signing in: any of the person's passkeys for this site.
    pub allow_credentials: Vec<PasskeyDescriptor>,
    pub user_verification: String,
}

/// A new passkey's answer (`AuthenticatorAttestationResponseJSON`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct PasskeyAttestationResponse {
    #[serde(rename = "clientDataJSON")]
    pub client_data_json: String,
    pub attestation_object: String,
    /// How the browser reaches the authenticator: `internal`, `hybrid`,
    /// `usb`…
    pub transports: Option<Vec<String>>,
}

/// What `navigator.credentials.create` returned, in its JSON form
/// (`RegistrationResponseJSON`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct PasskeyRegistration {
    pub id: String,
    pub raw_id: String,
    pub response: PasskeyAttestationResponse,
    pub authenticator_attachment: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
}

/// A passkey's signature (`AuthenticatorAssertionResponseJSON`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct PasskeyAssertionResponse {
    #[serde(rename = "clientDataJSON")]
    pub client_data_json: String,
    pub authenticator_data: String,
    pub signature: String,
    /// The account the passkey belongs to (`PasskeyUser.id`).
    pub user_handle: Option<String>,
}

/// What `navigator.credentials.get` returned, in its JSON form
/// (`AuthenticationResponseJSON`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(rename_all = "camelCase")]
pub struct PasskeyAssertion {
    pub id: String,
    pub raw_id: String,
    pub response: PasskeyAssertionResponse,
    pub authenticator_attachment: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
}

/// `POST /api/v1/me/passkeys/options`. Making a passkey takes the password
/// unless this browser signed in within the last 15 minutes; without a
/// password, a code from the authenticator app.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
#[serde(default)]
pub struct PasskeyOptionsRequest {
    pub password: Option<String>,
    pub code: Option<String>,
}

/// `POST /api/v1/me/passkeys`: keeps the passkey just made.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AddPasskeyRequest {
    pub credential: PasskeyRegistration,
    /// Its name when its provider doesn't say: the device, such as "Chrome
    /// on macOS".
    pub name: Option<String>,
}

/// `PATCH /api/v1/me/passkeys/{id}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UpdatePasskeyRequest {
    pub name: String,
}

/// `POST /api/v1/auth/passkey`: signs in with a passkey.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PasskeySignInRequest {
    pub credential: PasskeyAssertion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum TokenKind {
    /// A browser session.
    Web,
    /// Issued to the CLI through the device flow.
    Cli,
    /// A personal access token.
    Pat,
}

impl TokenKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Cli => "cli",
            Self::Pat => "pat",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "web" => Some(Self::Web),
            "cli" => Some(Self::Cli),
            "pat" => Some(Self::Pat),
            _ => None,
        }
    }
}

/// What a limited access token does in fctl, within what its person may
/// (`Read` < `Run` < `Full`).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum TokenAccess {
    /// Reads projects and sessions, as a viewer: their transcripts,
    /// changes, memory and retrospectives. Changes nothing, starts nothing.
    Read,
    /// Also starts sessions, messages and interrupts their agents, stops
    /// and resumes the sessions its person started, and keeps projects'
    /// memory, as an editor.
    Run,
    /// Everything its person may do in its projects.
    #[default]
    Full,
}

impl TokenAccess {
    pub const ALL: [Self; 3] = [Self::Read, Self::Run, Self::Full];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Run => "run",
            Self::Full => "full",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "read" => Some(Self::Read),
            "run" => Some(Self::Run),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

/// What a limited access token may do. A token without one does everything
/// its person may, as tokens did before; one with one reaches projects and
/// sessions only, never the account, workspaces' people and settings,
/// other tokens or administration.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct TokenScope {
    pub access: TokenAccess,
    /// Where it reaches: absent, every project its person sees, those to
    /// come too.
    pub places: Option<TokenPlaces>,
    /// The most the agents of the sessions it starts, messages or resumes
    /// may do on GitHub (`CreateSessionRequest::github_access`). Absent:
    /// push.
    pub github_access: Option<GithubAccess>,
}

/// The projects a limited access token reaches.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TokenPlaces {
    /// Workspaces whose every project it reaches, those made later too.
    pub workspaces: Vec<TokenPlace>,
    /// Projects it reaches.
    pub projects: Vec<TokenPlace>,
}

/// A workspace or project a token reaches, as it is now.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct TokenPlace {
    pub id: String,
    /// Absent once it is gone or its person no longer sees it: the token
    /// reaches nothing there.
    pub slug: Option<String>,
    pub name: Option<String>,
    /// A project's workspace, by name.
    pub workspace: Option<String>,
}

/// A web session, CLI token or personal access token (never the secret).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct TokenInfo {
    pub id: TokenId,
    pub kind: TokenKind,
    pub name: Option<String>,
    pub created_at: Millis,
    pub last_used_at: Option<Millis>,
    pub expires_at: Option<Millis>,
    pub user_agent: Option<String>,
    /// The token used for this request.
    pub current: bool,
    /// What an access token is limited to. Absent: it does everything its
    /// person may (browser sessions and the CLI's sign-ins always do).
    pub scope: Option<TokenScope>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TokenList {
    pub tokens: Vec<TokenInfo>,
}

/// `POST /api/v1/auth/tokens`: a personal access token. With none of
/// `access`, `workspaces`, `projects` and `github_access`, it does
/// everything its person may; with any, it is limited ([`TokenScope`]).
/// Control planes from before limits ignore them and make a token without
/// any: `fctl token` checks the answer's `scope`.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateTokenRequest {
    pub name: String,
    pub expires_in_days: Option<u32>,
    /// What it does in fctl. Absent: `full`.
    pub access: Option<TokenAccess>,
    /// Workspaces (ids or slugs) whose every project it reaches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
    pub workspaces: Vec<String>,
    /// Projects (ids or slugs) it reaches. With neither these nor
    /// `workspaces`, every project its person sees.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
    pub projects: Vec<String>,
    /// The most the agents of its sessions may do on GitHub. Absent: push.
    pub github_access: Option<GithubAccess>,
}

impl CreateTokenRequest {
    /// Whether it asks for any limit.
    pub fn limited(&self) -> bool {
        self.access.is_some_and(|a| a != TokenAccess::Full)
            || !self.workspaces.is_empty()
            || !self.projects.is_empty()
            || self.github_access.is_some_and(|g| g != GithubAccess::Push)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreatedToken {
    pub token: TokenInfo,
    /// Shown once.
    pub secret: String,
}

// ───────────────────────────── device flow (CLI sign-in) ─────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeviceStartRequest {
    /// Shown to the person approving, e.g. "fctl on carls-macbook".
    pub client_name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeviceStartResponse {
    pub device_code: String,
    /// Typed or confirmed in the browser, e.g. `KFTW-QXPL`.
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u32,
    /// Seconds between polls.
    pub interval: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DevicePollRequest {
    pub device_code: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DevicePollResponse {
    Pending,
    /// Polling too fast; wait `interval` more seconds.
    SlowDown,
    Denied,
    Expired,
    Approved {
        token: String,
        me: Box<Me>,
    },
    /// Approved, for a CLI that asked its person to confirm it's them
    /// (`POST /api/v1/auth/verify/device`): its token counts as confirmed
    /// until then.
    Verified {
        verified_until: Millis,
    },
}

/// `GET /api/v1/auth/device/{user_code}`: what the approval page shows.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct DeviceInfo {
    pub user_code: String,
    pub client_name: String,
    pub created_at: Millis,
    pub expires_at: Millis,
    pub ip: Option<String>,
    pub location: Option<String>,
    pub user_agent: Option<String>,
    /// The CLI is signed in already, as you, and asks you to confirm it's
    /// you for a sensitive action (`POST /api/v1/auth/verify/device`):
    /// approving it confirms that CLI for 15 minutes instead of signing one
    /// in.
    #[serde(default)]
    pub verify: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeviceDecisionRequest {
    pub approve: bool,
}

// ───────────────────────────── admin ─────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UserList {
    pub users: Vec<User>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateUserRequest {
    pub role: Option<InstanceRole>,
    pub status: Option<UserStatus>,
    pub name: Option<String>,
}

/// Who may sign up: an exact email (`ada@example.com`) or a domain
/// (`*@example.com`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AllowEntry {
    pub id: AllowId,
    pub pattern: String,
    /// Role given to people who sign up through this entry.
    pub role: InstanceRole,
    pub note: Option<String>,
    pub created_by: Option<UserRef>,
    pub created_at: Millis,
    /// For exact emails: the account, once it exists.
    pub user_id: Option<UserId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AllowList {
    pub entries: Vec<AllowEntry>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateAllowRequest {
    pub pattern: String,
    pub role: Option<InstanceRole>,
    pub note: Option<String>,
    /// Also create an invitation link (exact emails only).
    pub invite: Option<bool>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreatedAllow {
    pub entry: AllowEntry,
    pub invite: Option<Invite>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Invite {
    pub id: InviteId,
    pub email: String,
    /// The link to share (contains the secret; shown once).
    pub url: Option<String>,
    pub role: InstanceRole,
    pub expires_at: Millis,
    /// Whether it was emailed.
    pub emailed: bool,
    /// The workspaces it invites to as well.
    #[serde(default)]
    pub workspaces: Vec<WorkspaceRef>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateInviteRequest {
    pub email: String,
    pub role: Option<InstanceRole>,
    /// Workspaces to invite them to as well, ones you are an admin of: they
    /// join them with the account they create, or with the one they connect
    /// this address to.
    pub workspaces: Option<Vec<InviteWorkspace>>,
}

/// A workspace an instance invitation also invites to.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct InviteWorkspace {
    /// Its id or slug.
    pub workspace: String,
    /// `member` when absent.
    pub role: Option<WorkspaceRole>,
}

/// `GET /api/v1/invites/{token}` (public): who an invitation is for.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct InvitePreview {
    pub email: String,
    pub instance: String,
    pub invited_by: Option<String>,
    pub expires_at: Millis,
    /// The workspaces it invites to as well.
    #[serde(default)]
    pub workspaces: Vec<WorkspaceRef>,
    /// The address asked for early access, which this invitation answers.
    #[serde(default)]
    pub early_access: bool,
}

/// `POST /api/v1/access-requests` (public, where [`Features::early_access`]):
/// someone without an account asks for early access. The answer
/// ([`AccessRequested`]) is the same whether the address asked before, was
/// invited or has an account.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RequestAccessRequest {
    pub email: String,
}

/// The answer to [`RequestAccessRequest`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AccessRequested {
    /// Withdraws the request ([`WithdrawAccessRequest`]) when it was the
    /// address's first; otherwise it withdraws nothing, and looks the same.
    pub token: String,
}

/// `POST /api/v1/access-requests/withdraw` (public): the browser that asked
/// for early access withdraws its request. The answer is `ok` whether the
/// token withdrew one or not.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WithdrawAccessRequest {
    pub token: String,
}

/// Someone who asked for early access, for the instance's admins.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AccessRequest {
    pub id: AccessRequestId,
    pub email: String,
    /// When they first asked.
    pub created_at: Millis,
    /// When they last asked: asking again moves it.
    pub asked_at: Millis,
    /// When an admin last invited the address (from this list or any
    /// other way).
    pub invited_at: Option<Millis>,
    pub invited_by: Option<UserRef>,
    /// When the address's newest unused invitation stops working; absent
    /// once it has, or before any.
    pub invite_expires_at: Option<Millis>,
}

/// `GET /api/v1/admin/access-requests`: who asked for early access and
/// has no account yet, those still waiting first (longest waiting first),
/// then those invited (latest first). A request goes once its address has
/// an account, when an admin removes it or the browser that asked
/// withdraws it, or a year after it last asked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AccessRequestList {
    /// At most [`AccessRequestList::LIMIT`].
    pub requests: Vec<AccessRequest>,
    /// How many wait for an invitation, and how many were invited, in all.
    pub waiting: u32,
    pub invited: u32,
}

impl AccessRequestList {
    pub const LIMIT: u32 = 500;
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AuditEntry {
    pub id: String,
    pub ts: Millis,
    pub actor: Option<UserRef>,
    pub action: String,
    pub target: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub data: Option<serde_json::Value>,
    /// The address it came from: in a workspace's log, only for what the
    /// caller did.
    pub ip: Option<String>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AuditList {
    pub entries: Vec<AuditEntry>,
    pub next_before: Option<Millis>,
}

/// `GET /api/v1/admin/settings`: what admins decide for the whole instance.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct InstanceSettings {
    /// Projects may let their agents publish screenshots at links anyone
    /// who has one can open (each project turns it on in its settings:
    /// [`ProjectSettings::publish_screenshots`]). Off by default. Turning
    /// it off takes the published ones offline too; they are back if it is
    /// turned on again before they expire.
    pub publish_screenshots: bool,
    /// Everyone who signs in with a password needs a second factor too.
    /// People who haven't set one up can do nothing else until they have,
    /// in the browser or with their CLI and access tokens. People who sign
    /// in only with Google don't need one, unless `two_factor_google`.
    pub require_two_factor: bool,
    /// Google sign-ins ask for the second factor too, of everyone who has
    /// one (each person can also choose this for themselves). With
    /// `require_two_factor`, everyone needs one.
    pub two_factor_google: bool,
    /// Sensitive actions ask people to confirm it's them first, with a
    /// passkey or a code from their authenticator app, when they have one
    /// (`fctl_domain::verify::Sensitive` lists them: inviting and promoting
    /// admins, turning two-factor sign-in off, adding a runner to a
    /// workspace, new access tokens and CLI sign-ins, exporting data, and
    /// deleting accounts, projects and workspaces). Once confirmed, a
    /// browser, CLI or token isn't asked again for 15 minutes. Off by
    /// default; only the primary instance owner turns it on or off, and
    /// turning it off is one of those actions.
    pub verify_sensitive_actions: bool,
}

/// `PATCH /api/v1/admin/settings`: absent fields stay as they are.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateInstanceSettingsRequest {
    pub publish_screenshots: Option<bool>,
    pub require_two_factor: Option<bool>,
    pub two_factor_google: Option<bool>,
    /// Only the primary instance owner changes it.
    pub verify_sensitive_actions: Option<bool>,
}

// ───────────────────────────── workspaces ─────────────────────────────

/// The role every member of a workspace has in each of its projects, on top
/// of the role they have as the project's member.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum BaseRole {
    /// Members see the projects they are members of.
    #[default]
    None,
    /// Members see every project's sessions.
    Viewer,
    /// Members start sessions and message agents in every project.
    Editor,
    /// Members also change every project's settings, secrets and members.
    Maintainer,
}

impl BaseRole {
    pub const ALL: [Self; 4] = [Self::None, Self::Viewer, Self::Editor, Self::Maintainer];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Viewer => "viewer",
            Self::Editor => "editor",
            Self::Maintainer => "maintainer",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == s)
    }

    /// The project role it gives.
    pub fn project_role(self) -> Option<ProjectRole> {
        match self {
            Self::None => None,
            Self::Viewer => Some(ProjectRole::Viewer),
            Self::Editor => Some(ProjectRole::Editor),
            Self::Maintainer => Some(ProjectRole::Maintainer),
        }
    }
}

/// A workspace, as its members see it (`GET /api/v1/workspaces`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Workspace {
    pub id: WorkspaceId,
    /// URL-safe, unique per instance.
    pub slug: String,
    pub name: String,
    pub color: WorkspaceColor,
    /// Its avatar image, a path in the app; absent: its initial on `color`.
    pub avatar_url: Option<String>,
    /// Your role in it.
    pub role: WorkspaceRole,
    /// The address you are a member as: the one its people see, and whose
    /// commit email credits you in its projects' commits.
    pub email: String,
    /// The role every member has in every project.
    pub base_role: BaseRole,
    /// Members need two-factor sign-in: an authenticator app or a passkey.
    /// Its admins chose that, or the instance requires it
    /// (`instance_requires_two_factor`).
    pub require_two_factor: bool,
    /// The instance requires two-factor sign-in, so every workspace does
    /// too, and its admins can't turn it off
    /// ([`InstanceSettings::require_two_factor`]).
    #[serde(default)]
    pub instance_requires_two_factor: bool,
    /// It requires two-factor sign-in, which you haven't set up: until you
    /// do, its projects and sessions are closed to you.
    pub blocked: bool,
    /// The Claude connection your sessions in it run on. Absent: your
    /// default one.
    pub claude: Option<CredentialId>,
    /// The primary admin.
    pub primary: Option<UserRef>,
    pub members: u32,
    pub projects: u32,
    pub runners: u32,
    /// Its runners that are online now.
    pub runners_online: u32,
    /// The GitHub accounts it works in.
    pub github: Vec<String>,
    /// Its admins skipped inviting people while setting it up, as in a
    /// workspace for one: the app's setup checklist leaves that step out.
    #[serde(default)]
    pub skip_invites: bool,
    /// Your sessions in it run only on runners you registered: not on the
    /// ones of its other people, which see a session's Claude token. False
    /// from control planes before it.
    #[serde(default)]
    pub own_runners: bool,
    pub created_at: Millis,
}

impl Workspace {
    pub fn to_ref(&self) -> WorkspaceRef {
        WorkspaceRef {
            id: self.id.clone(),
            slug: self.slug.clone(),
            name: self.name.clone(),
            color: self.color,
            avatar_url: self.avatar_url.clone(),
        }
    }
}

/// `GET /api/v1/workspaces`: yours, and the invitations waiting for you.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WorkspaceList {
    pub workspaces: Vec<Workspace>,
    pub invitations: Vec<WorkspaceInvitation>,
    /// The most workspaces one person can be a member of.
    pub limit: u32,
}

/// `POST /api/v1/workspaces`: you become its primary admin.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateWorkspaceRequest {
    pub name: String,
    /// Absent: made from the name.
    pub slug: Option<String>,
    pub color: Option<WorkspaceColor>,
    /// The address to be a member as, one of yours. Absent: your primary one.
    pub email: Option<String>,
}

/// `PATCH /api/v1/workspaces/{id}` (admins): absent fields stay as they are.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateWorkspaceRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub color: Option<WorkspaceColor>,
    pub base_role: Option<BaseRole>,
    /// Members need two-factor sign-in. Not `false` while the instance
    /// requires it: every workspace does then.
    pub require_two_factor: Option<bool>,
    /// Leave inviting people out of the app's setup checklist, or (`false`)
    /// ask for it again.
    pub skip_invites: Option<bool>,
}

/// Someone in a workspace.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct WorkspaceMember {
    pub user: UserRef,
    /// The address they are a member as.
    pub email: String,
    pub role: WorkspaceRole,
    /// They have two-factor sign-in: an authenticator app or a passkey.
    /// For the workspace's admins only.
    pub two_factor: Option<bool>,
    pub added_by: Option<UserRef>,
    pub joined_at: Millis,
}

/// `GET /api/v1/workspaces/{id}/members`: its members, and the invitations
/// nobody has answered yet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WorkspaceMemberList {
    pub members: Vec<WorkspaceMember>,
    pub invitations: Vec<WorkspaceInvitation>,
}

/// An invitation to join a workspace, for an email address.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct WorkspaceInvitation {
    pub id: WorkspaceInviteId,
    pub workspace: WorkspaceRef,
    pub email: String,
    pub role: WorkspaceRole,
    pub invited_by: Option<UserRef>,
    pub created_at: Millis,
    pub expires_at: Millis,
    /// The workspace requires two-factor sign-in, which the invited person
    /// hasn't set up: joining waits for it.
    pub needs_two_factor: bool,
    /// Whether it was emailed (when it was just made).
    pub emailed: Option<bool>,
}

/// `POST /api/v1/workspaces/{id}/invitations` (admins).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct InviteToWorkspaceRequest {
    /// An address on the instance: one an account has, or one its
    /// allowlist admits.
    pub email: String,
    /// `member` when absent; `admin` makes them an admin.
    pub role: Option<WorkspaceRole>,
}

/// `PATCH /api/v1/workspaces/{id}/members/{user}` (admins): `admin` or
/// `member`. The primary admin hands their role over instead.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UpdateWorkspaceMemberRequest {
    pub role: WorkspaceRole,
}

/// Hands the primary admin role over, of a workspace
/// (`POST /api/v1/workspaces/{id}/transfer`) or of the instance
/// (`POST /api/v1/admin/transfer`): to someone who is an admin already. The
/// one who hands it over stays an admin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TransferRequest {
    pub user_id: UserId,
}

/// `PATCH /api/v1/workspaces/{id}/membership`: your own preferences there.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateMembershipRequest {
    /// The Claude connection your sessions there run on; an empty string
    /// goes back to your default one.
    pub claude: Option<String>,
    /// Its projects in the order you want them (ids or slugs), in lists
    /// and the app's sidebar: the ones named first, then the others as
    /// they were.
    pub project_order: Option<Vec<String>>,
    /// Run your sessions there only on runners you registered
    /// ([`Workspace::own_runners`]). Turning it on stops the ones running
    /// on other people's runners; they resume on yours.
    pub own_runners: Option<bool>,
}

/// `PUT /api/v1/me/workspace-order`: your workspaces in the order you want
/// them (ids or slugs), in lists and the app's sidebar: the ones named
/// first, then the others as they were. Returns them in their new order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct WorkspaceOrderRequest {
    pub workspaces: Vec<String>,
}

/// `DELETE /api/v1/workspaces/{id}` (its primary admin) and
/// `DELETE /api/v1/admin/workspaces/{id}` (instance admins): the name,
/// typed to confirm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeleteWorkspaceRequest {
    pub name: String,
}

/// Whether a workspace may use a GitHub account yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum GithubConnectionStatus {
    /// Its projects use the account's repositories.
    Active,
    /// One of its admins asked an instance admin, who hasn't approved it:
    /// only from control planes before workspaces connected accounts
    /// themselves.
    Requested,
}

impl GithubConnectionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Requested => "requested",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "requested" => Some(Self::Requested),
            _ => None,
        }
    }
}

/// A GitHub account a workspace works in: one the instance's GitHub App is
/// installed on.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct GithubConnection {
    /// The account's login.
    pub account: String,
    /// The repositories of it the workspace's projects may use. Absent:
    /// every one the app reaches there.
    pub repos: Option<Vec<String>>,
    pub status: GithubConnectionStatus,
    /// The workspace's admin who connected it (or asked an instance admin
    /// to, before workspaces connected accounts themselves).
    pub requested_by: Option<UserRef>,
    /// The instance admin who approved it, for connections from before
    /// workspaces connected accounts themselves.
    pub approved_by: Option<UserRef>,
    /// The GitHub account its admin signed in to GitHub as, which showed
    /// fctl what they may connect. Absent for connections from before that.
    pub github_user: Option<String>,
    pub created_at: Millis,
}

/// `GET /api/v1/workspaces/{id}/github`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct WorkspaceGithub {
    /// The instance has a GitHub App.
    pub configured: bool,
    pub app_slug: Option<String>,
    /// Installs the app on another account, or gives it more repositories,
    /// and comes back here through a GitHub sign-in, after which you choose
    /// what the workspace works in: for the workspace's admins.
    pub install_url: Option<String>,
    pub connections: Vec<GithubConnection>,
    /// Always false: connecting an account takes signing in to GitHub
    /// instead (older control planes asked an instance admin to approve
    /// it).
    pub approval_required: bool,
    /// Accounts can be connected here: the instance signs people in to
    /// GitHub with its GitHub App's client ID and secret. Without them,
    /// the workspace keeps the accounts it has.
    #[serde(default)]
    pub can_connect: bool,
    /// What your GitHub sign-ins showed you may connect here, while you
    /// choose. Only yours, for the workspace's admins.
    #[serde(default)]
    pub offers: Vec<GithubOffer>,
}

/// What you may connect of a GitHub account to a workspace, as the GitHub
/// sign-in you just made showed it, until `expires_at`:
/// `POST /api/v1/workspaces/{id}/github/offers/{account}` connects what you
/// choose of it ([`ChooseGithubRepos`]), and
/// `DELETE /api/v1/workspaces/{id}/github/offers/{account}` nothing.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct GithubOffer {
    /// The account's login.
    pub account: String,
    /// `Organization` or `User`.
    pub account_type: Option<String>,
    /// The GitHub account you signed in to GitHub as.
    pub github_user: String,
    /// You may connect every repository of it, also those the app reaches
    /// there later: it is your own account, or an organization whose every
    /// repository the app reaches and you are an admin of.
    pub all: bool,
    /// The repositories of it you may connect: those the app reaches that
    /// you are an admin of, in alphabetical order.
    pub repos: Vec<String>,
    /// How many more repositories the app reaches there that you can see
    /// but aren't an admin of.
    pub others: u32,
    pub expires_at: Millis,
}

/// `POST /api/v1/workspaces/{id}/github/offers/{account}` (admins): connect
/// what you choose of an account your GitHub sign-in offered you
/// ([`GithubOffer`]). The answer is the workspace's [`WorkspaceGithub`].
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ChooseGithubRepos {
    /// These of its repositories, from the offer's. Absent: every
    /// repository, also those the app reaches there later, where the offer
    /// has it.
    pub repos: Option<Vec<String>>,
}

/// `POST /api/v1/workspaces/{id}/github/connect` (admins): connect an
/// account the instance's GitHub App is installed on. The answer is a link
/// ([`ConnectGithubLink`]) to a GitHub sign-in that shows fctl what you
/// may connect there.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ConnectGithubRequest {
    /// The account's login. Absent: every account GitHub shows you the app
    /// on, to choose from.
    pub account: Option<String>,
    /// Only these repositories of it, connected once you have signed in
    /// as far as GitHub says you may. Absent: you choose from the ones you
    /// may connect after signing in ([`GithubOffer`]).
    pub repos: Option<Vec<String>>,
}

/// Where connecting an account goes on: open it in a browser signed in to
/// fctl as you. GitHub asks you to sign in, which shows fctl what you may
/// connect: every repository of your own account, or of an organization
/// whose every repository the app reaches and you are an admin of;
/// otherwise the repositories you are an admin of. The browser comes back
/// to the workspace's GitHub tab, where you choose from it, unless you
/// named the repositories.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ConnectGithubLink {
    pub url: String,
    /// It works until then.
    pub expires_at: Millis,
}

/// `GET /api/v1/admin/github` (instance admins): the accounts the
/// instance's GitHub App is installed on, and the workspaces that use
/// each. Workspaces' admins connect them; instance admins can take one
/// from a workspace.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct GithubInstallations {
    pub configured: bool,
    pub app_slug: Option<String>,
    /// Where to install the app on more accounts: only from older control
    /// planes, since workspaces install it themselves.
    pub install_url: Option<String>,
    pub installations: Vec<GithubInstallation>,
}

/// An account the instance's GitHub App is installed on, or that a
/// workspace names although the app isn't installed there (any more).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct GithubInstallation {
    pub account: String,
    /// `Organization` or `User`.
    pub account_type: Option<String>,
    /// Absent: the app isn't installed there.
    pub installation_id: Option<u64>,
    /// `all` or `selected`: the repositories the account gave the app.
    pub repository_selection: Option<String>,
    pub installed_at: Option<Millis>,
    pub workspaces: Vec<GithubWorkspaceConnection>,
}

/// A workspace that uses an account.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct GithubWorkspaceConnection {
    pub workspace: WorkspaceRef,
    pub repos: Option<Vec<String>>,
    pub status: GithubConnectionStatus,
    /// Its admin who connected it.
    pub requested_by: Option<UserRef>,
    /// The GitHub account they signed in to GitHub as.
    pub github_user: Option<String>,
    pub created_at: Millis,
}

/// A workspace, as instance admins see it: what it is and who runs it,
/// not what is in it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AdminWorkspace {
    pub workspace: WorkspaceRef,
    pub primary: Option<UserRef>,
    pub members: u32,
    pub projects: u32,
    pub runners: u32,
    pub github: Vec<String>,
    /// Members need two-factor sign-in: its admins chose that, or the
    /// instance requires it.
    pub require_two_factor: bool,
    pub created_at: Millis,
    /// Your role in it, when you are a member.
    pub role: Option<WorkspaceRole>,
    /// Its primary admin's account is disabled or gone, so nobody can hand
    /// the role over: an instance admin gives it to one of `candidates`.
    #[serde(default)]
    pub needs_primary: bool,
    /// Its members with active accounts, admins first, when it needs a
    /// primary admin.
    #[serde(default)]
    pub candidates: Vec<UserRef>,
}

/// `GET /api/v1/admin/workspaces`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AdminWorkspaceList {
    pub workspaces: Vec<AdminWorkspace>,
}

// ───────────────────────────── runners ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RunnerStatus {
    Online,
    Offline,
    Disabled,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Runner {
    pub id: RunnerId,
    pub name: String,
    pub status: RunnerStatus,
    pub version: Option<String>,
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub driver: Option<String>,
    pub drivers: Vec<DriverInfo>,
    pub capacity: u32,
    pub active_sessions: u32,
    /// The machine's CPUs and memory (MiB).
    pub cpus: Option<u32>,
    pub memory_mb: Option<u64>,
    /// Free memory and disk (MiB) at its last heartbeat, while it is connected.
    pub memory_available_mb: Option<u64>,
    pub disk_available_mb: Option<u64>,
    /// The free disk (MiB) its host keeps: while `disk_available_mb` is
    /// below it, no new session starts on it. Absent when it keeps none,
    /// and from older hosts.
    pub min_free_disk_mb: Option<u64>,
    /// What it gives each sandbox (absent from older hosts).
    pub sandbox: Option<SandboxLimits>,
    /// The image it gives sessions whose project sets none.
    pub image: Option<String>,
    /// The other images it runs, as its hello says ([`HostHello`]).
    /// Absent: any image.
    ///
    /// [`HostHello`]: crate::runner::HostHello
    pub allowed_images: Option<Vec<String>>,
    /// The repositories it builds images from. Absent: none.
    pub allowed_builds: Option<Vec<String>>,
    pub labels: Vec<String>,
    pub last_seen_at: Option<Millis>,
    pub created_at: Millis,
    pub created_by: Option<UserRef>,
    /// Set while the runner drains: no new session starts on it, and the
    /// sessions it has carry on there. Absent otherwise, and from control
    /// planes that can't drain runners.
    pub drain: Option<RunnerDrain>,
    /// The workspaces whose sessions it runs, of the ones you can see.
    /// Sessions start only on runners of their workspace. Absent from
    /// control planes before workspaces, which may still answer the web
    /// app while an instance upgrades.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<WorkspaceRef>>", optional))]
    pub workspaces: Vec<WorkspaceRef>,
    /// Whether you may add it to your workspaces and remove it from any:
    /// you registered it.
    #[serde(default)]
    pub yours: bool,
    /// Of `workspaces`, the ones where it runs only its owner's sessions:
    /// they added it for their own. In the others it runs everyone's.
    /// Absent from control planes before it.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<WorkspaceId>>", optional))]
    pub personal: Vec<WorkspaceId>,
}

/// A runner that drains.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct RunnerDrain {
    /// When it started draining.
    pub since: Millis,
    /// The admin who drained it. Absent when its host did (`fctl host drain`).
    pub by: Option<UserRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RunnerList {
    pub runners: Vec<Runner>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateRunnerRequest {
    pub name: String,
    /// The workspace (id or slug) whose sessions it runs. Absent: the one
    /// workspace you are an admin of, else the one you are a member of.
    pub workspace: Option<String>,
    /// It runs only your sessions in `workspace`. Absent: only yours when
    /// you are a member there, everyone's when you are an admin; only
    /// admins add runners for everyone's.
    pub personal: Option<bool>,
}

/// `PUT /api/v1/workspaces/{id}/runners/{runner}`: whose sessions there
/// your runner runs. Without a body, as with `personal` absent.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AddRunnerRequest {
    /// Only your sessions there. Absent: only yours when you are a member,
    /// everyone's when you are an admin; only admins add runners for
    /// everyone's.
    pub personal: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RunnerRegistration {
    pub runner: Runner,
    /// `fctl_rt_…`; shown once. Goes into the host's config.
    pub token: String,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateRunnerRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    /// Drain the runner (no new sessions start on it), or stop draining it.
    pub draining: Option<bool>,
}

/// `POST /api/v1/runner/drain`, with the runner token: a host drains itself,
/// or stops draining.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct DrainRequest {
    pub draining: bool,
    /// Also stop the sessions it runs now (`fctl host uninstall`): their
    /// agents stop and push their work, and from then on the sessions
    /// whose sandboxes it has resume on other runners.
    #[serde(default)]
    pub stop_sessions: Option<bool>,
}

/// `GET /api/v1/runner`, with the runner token: the runner as the control
/// plane sees it, and the sessions it runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RunnerDetail {
    pub runner: Runner,
    pub sessions: Vec<RunnerSession>,
}

/// A session a runner runs.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct RunnerSession {
    pub id: SessionId,
    pub project_slug: String,
    pub number: u32,
    pub title: String,
    pub status: SessionStatus,
    /// When a paused session resumes on its own.
    pub resume_at: Option<Millis>,
}

// ───────────────────────────── projects ─────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProjectList {
    pub projects: Vec<Project>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateRepoRequest {
    /// Repository name.
    pub name: String,
    pub private: bool,
    pub description: Option<String>,
    /// The organization to create it in, one the workspace works in.
    /// Absent: the instance's default organization when the workspace works
    /// in it, else the workspace's first organization.
    pub owner: Option<String>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateProjectRequest {
    /// The workspace (id or slug). Absent: your only workspace.
    pub workspace: Option<String>,
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    /// An existing repository, `owner/name`.
    pub repo: Option<String>,
    /// Or create a new one.
    pub create_repo: Option<CreateRepoRequest>,
    pub default_branch: Option<String>,
    pub settings: Option<ProjectSettings>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateProjectRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    /// `owner/name`, or `""` to unlink.
    pub repo: Option<String>,
    pub default_branch: Option<String>,
    /// Replaces all settings.
    pub settings: Option<ProjectSettings>,
    pub archived: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProjectMember {
    pub user: UserRef,
    pub email: String,
    pub role: ProjectRole,
    pub added_at: Millis,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MemberList {
    pub members: Vec<ProjectMember>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AddMemberRequest {
    pub email: String,
    pub role: ProjectRole,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SetRoleRequest {
    pub role: ProjectRole,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SecretInfo {
    pub name: String,
    pub updated_at: Millis,
    pub updated_by: Option<UserRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SecretList {
    pub secrets: Vec<SecretInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SetSecretRequest {
    pub value: String,
}

// ───────────────────────────── project repositories ─────────────────────────────

/// A repository a project's agents may use. All of a project's repositories
/// belong to one GitHub account, so one installation of the GitHub App mints
/// their tokens.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ProjectRepo {
    pub owner: String,
    /// Absent for an owner-wide grant: every repository of `owner` the
    /// GitHub App can reach.
    pub name: Option<String>,
    pub access: RepoAccess,
    /// The project's primary repository, cloned into each agent's working
    /// directory. Agents clone the others when they need them.
    pub primary: bool,
    pub added_by: Option<UserRef>,
    pub added_at: Option<Millis>,
}

/// `GET /api/v1/projects/{id}/repos`: the primary repository first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProjectRepoList {
    pub repos: Vec<ProjectRepo>,
}

/// `POST /api/v1/projects/{id}/repos`: adds a repository, or changes its
/// access. `DELETE /api/v1/projects/{id}/repos/{owner}/{name}` (`*` for an
/// owner-wide grant) removes one.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AddProjectRepoRequest {
    /// `owner/name`, or `owner/*` for every repository of `owner` the GitHub
    /// App can reach.
    pub repo: String,
    /// `read` when absent. Owner-wide grants are always read-only.
    pub access: Option<RepoAccess>,
}

// ───────────────────────────── sessions ─────────────────────────────

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SessionList {
    pub sessions: Vec<Session>,
    /// Pass as `?before=` for the next page.
    pub next_before: Option<Millis>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateSessionRequest {
    /// The task. May be empty when `attachments` are given.
    pub prompt: String,
    pub title: Option<String>,
    /// Branch to start from (default: the project's default branch).
    pub base_branch: Option<String>,
    /// Branch to work on (default: `fctl/<number>-<slug of title>`).
    pub branch: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Files to send with it: ids from `POST /api/v1/projects/{id}/attachments`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<AttachmentId>>", optional))]
    pub attachments: Vec<AttachmentId>,
    /// What its agent may do on GitHub: at most what its project's
    /// settings and the access token starting it allow, which is what it
    /// gets when absent. Asking for more fails.
    pub github_access: Option<GithubAccess>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateSessionRequest {
    pub title: Option<String>,
    /// What the session is about; an empty string clears it.
    pub description: Option<String>,
    pub archived: Option<bool>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct SendMessageRequest {
    /// May be empty when `attachments` are given.
    pub text: String,
    /// Idempotency key; retries with the same key deliver once.
    pub client_id: Option<String>,
    /// Files to send with it: ids from `POST /api/v1/sessions/{id}/attachments`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<AttachmentId>>", optional))]
    pub attachments: Vec<AttachmentId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SendMessageResponse {
    pub delivery_id: DeliveryId,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ResumeSessionRequest {
    pub prompt: Option<String>,
    /// Files to send with the prompt: ids from `POST /api/v1/sessions/{id}/attachments`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<AttachmentId>>", optional))]
    pub attachments: Vec<AttachmentId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EventPage {
    pub events: Vec<SessionEvent>,
    pub has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionShare {
    pub user: UserRef,
    pub email: String,
    pub role: SessionRole,
    pub created_at: Millis,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ShareList {
    pub shares: Vec<SessionShare>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreateShareRequest {
    pub email: String,
    pub role: SessionRole,
}

// ───────────────────────────── project memory ─────────────────────────────

/// `GET /api/v1/projects/{id}/memories`: the project's memory, its current
/// notes first, newest first, then the stale ones, most recently stale
/// first. With `?session=`, only the notes that session's agent wrote or
/// marked stale.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct MemoryList {
    pub memories: Vec<Memory>,
    /// How many current notes the project has, `?session=` or not.
    pub current: Option<u32>,
}

/// `POST /api/v1/projects/{id}/memories` (editors): a note from a person
/// for the project's agents; with `replaces`, in place of that note, which
/// goes stale, and with `also_replaces` in place of those too.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AddMemoryRequest {
    pub text: String,
    pub replaces: Option<MemoryId>,
    /// More notes it takes the place of. Control planes from before it
    /// replace only `replaces`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<MemoryId>>", optional))]
    pub also_replaces: Vec<MemoryId>,
}

/// `PATCH /api/v1/memories/{id}` (editors): absent fields stay as they
/// are. `DELETE /api/v1/memories/{id}` deletes one.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateMemoryRequest {
    pub text: Option<String>,
    /// `true` marks it stale, with `reason`; `false` makes it current
    /// again.
    pub stale: Option<bool>,
    pub reason: Option<String>,
    /// Marks it stale as a duplicate of that note, a current one of the
    /// same project, with `reason` or `Duplicate of mem_…`.
    pub duplicate_of: Option<MemoryId>,
}

// ───────────────────────────── retrospectives ─────────────────────────────

/// `GET /api/v1/projects/{id}/retrospectives` and
/// `GET /api/v1/sessions/{id}/retrospectives`: newest first.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct RetrospectiveList {
    pub retrospectives: Vec<Retrospective>,
    /// Pass as `?before=` for the next page: the id of the last one, which
    /// sort as they were recorded.
    pub next_before: Option<RetrospectiveId>,
    /// How many the project has of each outcome, whatever `?outcome=`
    /// chose. Only in the first page of the project's list.
    pub outcomes: Option<OutcomeCounts>,
}

/// How many retrospectives ended each way.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OutcomeCounts {
    pub completed: u32,
    pub partial: u32,
    pub blocked: u32,
}

// ───────────────────────────── review agents ─────────────────────────────

/// A project's review agents (`GET /api/v1/projects/{id}/review-agents`):
/// whether an agent reviews each new commit of the pull requests in its
/// repositories, on whose Claude subscription, and how.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ReviewAgents {
    pub enabled: bool,
    /// The person whose Claude subscription runs the reviews: who turned
    /// them on, or took them over since. Kept while they are off.
    pub operator: Option<UserRef>,
    /// The model they run on; absent: the project's.
    pub model: Option<String>,
    /// Their effort level; absent: the project's.
    pub effort: Option<String>,
    /// What they are told besides fctl's own review instructions.
    pub instructions: Option<String>,
    /// The repositories whose pull requests they review: those the
    /// project's agents may write to.
    pub repos: Vec<RepoRef>,
    /// Why no review starts now (they are on), or would start once they
    /// are on: no GitHub webhook, their person's Claude connection is gone…
    pub problem: Option<String>,
    pub updated_by: Option<UserRef>,
    pub updated_at: Option<Millis>,
}

/// `PATCH /api/v1/projects/{id}/review-agents`, for the project's owners
/// (and its workspace's admins), and the person they run for, who may
/// turn them off: absent fields stay as they are.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateReviewAgentsRequest {
    /// Turning them on runs them on the caller's Claude subscription.
    pub enabled: Option<bool>,
    /// `true`: run them on the caller's subscription from now on.
    pub run_as_me: Option<bool>,
    /// A model; `""` for the project's.
    pub model: Option<String>,
    /// An effort level; `""` for the project's.
    pub effort: Option<String>,
    /// `""` for none.
    pub instructions: Option<String>,
}

/// How a review went, or that it is still under way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    /// Waiting for a runner, or reviewing.
    Running,
    /// Done, with nothing to point out: its check passed.
    Passed,
    /// Done: it posted findings, or earlier ones are still open. Its check
    /// is neutral, since acting on them is up to the pull request's people.
    Commented,
    /// A newer commit came before it finished, and the review of that one
    /// took its place.
    Superseded,
    /// It ended without posting: someone stopped it, or its agent ended
    /// without a review.
    Stopped,
    /// It couldn't run, or couldn't post.
    Failed,
}

impl ReviewState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Commented => "commented",
            Self::Superseded => "superseded",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }

    /// What D1 keeps; a state this version doesn't know reads as failed.
    pub fn parse(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "passed" => Self::Passed,
            "commented" => Self::Commented,
            "superseded" => Self::Superseded,
            "stopped" => Self::Stopped,
            _ => Self::Failed,
        }
    }

    /// Whether it ended, one way or another.
    pub fn is_done(self) -> bool {
        self != Self::Running
    }
}

/// A review agent's pass over a pull request at one commit: its session,
/// what it reviewed and what came of it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Review {
    /// Its session, whose page shows the agent's work.
    pub session_id: SessionId,
    /// The session's number in its project (`#12`).
    pub session_number: u32,
    pub status: SessionStatus,
    pub state: ReviewState,
    /// Why it stopped or failed, or what came in its place.
    pub reason: Option<String>,
    pub repo: RepoRef,
    /// The pull request's number.
    pub number: u64,
    pub url: String,
    pub title: Option<String>,
    /// Who opened the pull request: their GitHub login.
    pub author: Option<String>,
    pub draft: bool,
    /// The commit it reviews.
    pub head_sha: String,
    /// Findings it posted.
    pub findings: u32,
    /// Earlier findings it marked fixed and resolved.
    pub resolved: u32,
    /// Findings of earlier reviews still open after it.
    pub still_open: u32,
    /// What it posted on GitHub: its summary, above the review of its
    /// findings; absent while it posted none.
    pub review_url: Option<String>,
    /// Why GitHub shows no check for it (the app can't write checks).
    pub check_error: Option<String>,
    /// On whose Claude subscription it ran.
    pub operator: UserRef,
    pub created_at: Millis,
    pub finished_at: Option<Millis>,
    /// How long its agent worked.
    pub busy_ms: u64,
}

/// `GET /api/v1/projects/{id}/reviews`: newest first. With `?session=`, that
/// session's review alone.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ReviewList {
    pub reviews: Vec<Review>,
    /// Pass as `?before=` for the next page.
    pub next_before: Option<Millis>,
}

/// `POST /api/v1/projects/{id}/reviews` (people who start sessions there,
/// while its review agents are on): review a pull request's head now, in
/// place of a review of it that is still under way.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StartReviewRequest {
    /// `owner/name#12`, its URL, or `12` in the project's repository.
    pub pull_request: String,
}

/// How much a finding matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// A bug or a security problem that will hurt.
    High,
    /// A likely problem, or a clear break of the repository's rules.
    Medium,
    /// Worth fixing, harmless meanwhile.
    Low,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

/// A problem a review agent found, on lines of the pull request.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ReviewFinding {
    /// The file, by its path in the repository.
    pub path: String,
    /// The line, in the pull request's version of the file (in the old
    /// one for `deleted` lines): the last one, for several.
    pub line: u32,
    /// The first line, for a finding about several.
    pub start_line: Option<u32>,
    /// About lines the pull request deletes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub deleted: bool,
    pub severity: Severity,
    /// What is wrong, in a few words.
    pub title: String,
    /// What goes wrong, when and where, in a few short sentences.
    pub body: String,
    /// The lines that fix it, in place of `start_line` to `line`, which
    /// GitHub offers to apply.
    pub suggestion: Option<String>,
}

/// An earlier finding the new commits fixed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResolvedFinding {
    /// Its review thread (`PRRT_…`), as `fctl review threads` lists it.
    pub thread: String,
    /// What changed, in a sentence.
    pub note: String,
}

/// One of the review threads fctl started on a pull request: an earlier
/// review's finding, with what people answered.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct FindingThread {
    /// GitHub's id (`PRRT_…`).
    pub id: String,
    pub path: Option<String>,
    pub line: Option<u32>,
    pub resolved: bool,
    /// The lines it is about changed since it was posted.
    pub outdated: bool,
    /// The finding, as posted.
    pub body: String,
    pub url: Option<String>,
    /// The answers in it, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<ThreadReply>>", optional))]
    pub replies: Vec<ThreadReply>,
}

/// An answer in a review thread.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ThreadReply {
    /// Their GitHub login.
    pub author: Option<String>,
    pub body: String,
}

/// What fctl posted for a review session (`fctl review submit`).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct PostedReview {
    pub state: ReviewState,
    /// Its summary on GitHub, above the review of its findings; absent
    /// when it had nothing to post.
    pub url: Option<String>,
    /// Findings posted on their lines.
    pub comments: u32,
    /// Findings whose lines the pull request doesn't change, posted in its
    /// summary instead.
    pub in_text: u32,
    /// The earlier findings marked fixed, by thread.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
    pub resolved: Vec<String>,
    /// Earlier findings still open.
    pub still_open: u32,
    /// It was posted already: this answers a second submission.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub again: bool,
    /// The session whose agent opened the pull request, which fctl sent
    /// the findings to, so that it decides what to do about them. Control
    /// planes from before it leave it out.
    pub sent_to: Option<SessionId>,
}

// ───────────────────────────── schedules ─────────────────────────────

/// A project's schedule (`GET /api/v1/projects/{id}/schedules`): a task
/// fctl starts a session for on its own, once at a set time or again and
/// again, on the Claude subscription of the person it runs for, its owner.
/// People run it when they choose too, and so do other tools with an
/// access token (`POST /api/v1/schedules/{id}/runs`), on their own
/// subscription.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Schedule {
    pub id: ScheduleId,
    pub project_id: ProjectId,
    pub name: String,
    /// What each run's agent is asked to do.
    pub prompt: String,
    /// When it runs again and again: a cron expression (minute, hour, day
    /// of the month, month and day of the week) in `timezone`. Absent for a
    /// schedule that runs once, or when asked.
    pub cron: Option<String>,
    /// When it runs once. Absent for one that runs again and again, or when
    /// asked. Without either, it runs only when someone runs it, or another
    /// tool does with an access token.
    pub at: Option<Millis>,
    /// The IANA time zone its times are in, such as `Europe/Stockholm`.
    pub timezone: String,
    /// What its sessions start with; absent: the project's.
    pub model: Option<String>,
    pub effort: Option<String>,
    pub base_branch: Option<String>,
    /// What its sessions' agents may do on GitHub; absent: as much as the
    /// project allows.
    pub github_access: Option<GithubAccess>,
    /// `false` while it is paused: it starts nothing until it is resumed,
    /// not even when asked.
    pub enabled: bool,
    pub state: ScheduleState,
    /// Why fctl paused it: its owner can no longer run it.
    pub paused_reason: Option<String>,
    /// Why its next run wouldn't start now, while it is on: its owner's
    /// plan is at its limit, or they can't start sessions here any more.
    pub problem: Option<String>,
    /// When it starts its next session; absent while it is paused, for one
    /// that runs when asked, and once one that runs once has run.
    pub next_run_at: Option<Millis>,
    /// Its latest run.
    pub last_run: Option<ScheduleRun>,
    /// Whose Claude subscription its runs start on: who set it up, or took
    /// it over since. Absent once their account is gone.
    pub owner: Option<UserRef>,
    pub created_by: Option<UserRef>,
    pub created_at: Millis,
    pub updated_by: Option<UserRef>,
    pub updated_at: Millis,
}

/// Where a schedule stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ScheduleState {
    /// It starts a session at its next time, or when asked.
    Active,
    /// It starts nothing until someone resumes it.
    Paused,
    /// It ran once, as it was set to.
    Done,
}

/// One time a schedule ran: the session it started, or why it started
/// none.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ScheduleRun {
    pub id: ScheduleRunId,
    pub schedule_id: ScheduleId,
    /// When it was due: the schedule's time, or when someone asked.
    pub due_at: Millis,
    /// When fctl started its session, or decided to start none.
    pub at: Millis,
    pub outcome: ScheduleRunOutcome,
    /// Why it was skipped, or failed.
    pub reason: Option<String>,
    pub trigger: ScheduleTrigger,
    /// Who it ran for: the owner, at the schedule's time; whoever asked,
    /// or whose access token did, otherwise.
    pub by: Option<UserRef>,
    /// The session it started.
    pub session: Option<SessionRef>,
    /// That session's status now.
    pub status: Option<SessionStatus>,
}

/// What came of a schedule's run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ScheduleRunOutcome {
    /// A session started.
    Started,
    /// No session started, on purpose: its owner's plan was at its limit,
    /// or its last run was still working.
    Skipped,
    /// No session started: something went wrong.
    Failed,
}

impl ScheduleRunOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    /// What D1 keeps; an outcome this version doesn't know reads as failed.
    pub fn parse(s: &str) -> Self {
        match s {
            "started" => Self::Started,
            "skipped" => Self::Skipped,
            _ => Self::Failed,
        }
    }
}

/// What started a schedule's run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ScheduleTrigger {
    /// Its time came.
    Time,
    /// Someone ran it, in the app or the CLI.
    Person,
    /// Another tool ran it, with an access token.
    Token,
}

impl ScheduleTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Time => "time",
            Self::Person => "person",
            Self::Token => "token",
        }
    }

    /// What D1 keeps; a trigger this version doesn't know reads as its time.
    pub fn parse(s: &str) -> Self {
        match s {
            "person" => Self::Person,
            "token" => Self::Token,
            _ => Self::Time,
        }
    }
}

/// `GET /api/v1/projects/{id}/schedules`: the project's schedules, in the
/// order they were made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScheduleList {
    pub schedules: Vec<Schedule>,
}

/// `GET /api/v1/schedules/{id}/runs`: newest first, before `?before=` for
/// the next page.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ScheduleRunList {
    pub runs: Vec<ScheduleRun>,
    /// Where the next page starts, when there is one.
    pub next_before: Option<Millis>,
}

/// `POST /api/v1/projects/{id}/schedules`, for people who start sessions
/// in the project. It runs on the caller's Claude subscription.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateScheduleRequest {
    /// Absent: the task's first line.
    pub name: Option<String>,
    pub prompt: String,
    /// When it runs again and again: give this or `at`, or neither for a
    /// schedule that runs only when asked.
    pub cron: Option<String>,
    /// When it runs once.
    pub at: Option<Millis>,
    /// Absent: UTC.
    pub timezone: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub base_branch: Option<String>,
    /// At most what the project allows; absent: that much.
    pub github_access: Option<GithubAccess>,
    /// `false`: it starts paused.
    pub enabled: Option<bool>,
}

/// `PATCH /api/v1/schedules/{id}`: absent fields stay as they are. Its
/// owner changes it, and so do the project's maintainers and owners, which
/// runs it on their own Claude subscription from then on. Pausing it is
/// open to both, and resuming it as changing it is.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateScheduleRequest {
    pub name: Option<String>,
    pub prompt: Option<String>,
    /// A cron expression: it runs again and again from now on; `""`, and no
    /// `at`, for only when asked.
    pub cron: Option<String>,
    /// A time: it runs once, then.
    pub at: Option<Millis>,
    pub timezone: Option<String>,
    /// `""` for the project's.
    pub model: Option<String>,
    /// `""` for the project's.
    pub effort: Option<String>,
    /// `""` for the project's default branch.
    pub base_branch: Option<String>,
    /// `push`, `comment` or `read`; `""` for as much as the project allows.
    pub github_access: Option<String>,
    /// `false` pauses it; `true` resumes it.
    pub enabled: Option<bool>,
    /// `true`: run it on the caller's Claude subscription from now on.
    pub run_as_me: Option<bool>,
}

/// `POST /api/v1/schedules/{id}/runs`: starts its task now, in a session on
/// the caller's Claude subscription, for people who start sessions in its
/// project and access tokens that run sessions there, unless it is paused.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct RunScheduleRequest {
    /// What this run's agent gets after the task: what another tool knows
    /// of why it runs, such as an issue's title and text. The agent reads it
    /// as information, not instructions.
    pub context: Option<String>,
}

// ───────────────────────────── what agents ask ─────────────────────────────

/// What an agent asks of the control plane about its session's project,
/// with `fctl memory`, `fctl retro`, `fctl resolve` and `fctl review` in its
/// sandbox. fctl-agent hands it to its host, which sends it with its runner
/// token (`POST /api/v1/runner/sessions/{id}/calls`) and hands the
/// [`AgentAnswer`] back. Each needs the project's setting:
/// `project_memory`, or `retrospectives`; resolving review threads needs a
/// session whose agent may comment, and reviews a review agent's session.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentCall {
    /// The project's memory as it is now: its current notes, and with
    /// `stale` the stale ones too.
    Memories {
        #[serde(default)]
        stale: bool,
    },
    /// A note for the agents of later sessions. With `replaces`, it takes
    /// the place of that note, which goes stale, and with `also_replaces`
    /// of those too.
    Remember {
        text: String,
        #[cfg_attr(feature = "ts", ts(optional))]
        replaces: Option<MemoryId>,
        /// More notes it takes the place of. Hosts and control planes from
        /// before it replace only `replaces`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(feature = "ts", ts(as = "Option<Vec<MemoryId>>", optional))]
        also_replaces: Vec<MemoryId>,
    },
    /// Marks a note stale: it turned out wrong, or out of date. With
    /// `duplicate_of`, it repeats that note, a current one, which stays;
    /// hosts and control planes from before it mark it stale with the
    /// reason alone.
    MarkStale {
        id: MemoryId,
        reason: String,
        #[cfg_attr(feature = "ts", ts(optional))]
        duplicate_of: Option<MemoryId>,
    },
    /// Records a retrospective of a piece of the session's work.
    Retrospective {
        outcome: RetrospectiveOutcome,
        worked_on: String,
        #[cfg_attr(feature = "ts", ts(optional))]
        approach: Option<String>,
        #[cfg_attr(feature = "ts", ts(optional))]
        challenges: Option<String>,
        #[cfg_attr(feature = "ts", ts(optional))]
        insights: Option<String>,
    },
    /// The retrospectives the session recorded, newest first, for the
    /// agent to find the one to update. fctl-agents and control planes
    /// from before it refuse it.
    Retrospectives,
    /// Updates a retrospective the session recorded, as a later turn
    /// carries on with its piece of work: the outcome and parts given
    /// replace those it had, an empty part leaves it out (what it worked
    /// on can't be empty), and the others stay. fctl-agents and control
    /// planes from before it refuse it.
    UpdateRetrospective {
        id: RetrospectiveId,
        #[cfg_attr(feature = "ts", ts(optional))]
        outcome: Option<RetrospectiveOutcome>,
        #[cfg_attr(feature = "ts", ts(optional))]
        worked_on: Option<String>,
        #[cfg_attr(feature = "ts", ts(optional))]
        approach: Option<String>,
        #[cfg_attr(feature = "ts", ts(optional))]
        challenges: Option<String>,
        #[cfg_attr(feature = "ts", ts(optional))]
        insights: Option<String>,
    },
    /// Resolves a review thread of a pull request in a repository the
    /// session may write to, or with `unresolve` opens it again. GitHub
    /// lets only tokens that can push resolve threads, so the control plane
    /// does it for agents that may comment but not push, with a token it
    /// keeps.
    ResolveThread {
        /// GitHub's id of the thread (`PRRT_…`).
        thread: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
        unresolve: bool,
    },
    /// The review threads fctl started on the pull request a review
    /// session reviews (`fctl review threads`): earlier reviews' findings,
    /// whether they are resolved, and what people answered. Only for review
    /// sessions; fctl-agents and control planes from before review agents
    /// refuse it.
    ReviewThreads,
    /// Posts a review session's review, once (`fctl review submit`): a
    /// summary, then its findings as comments on their lines, in one
    /// review, and the earlier findings the new commits fixed, which fctl
    /// marks fixed and resolves.
    /// Without findings it posts nothing new and passes the check. Only
    /// for review sessions.
    SubmitReview {
        /// A sentence or two on the pull request as a whole, for the
        /// review's summary.
        #[cfg_attr(feature = "ts", ts(optional))]
        summary: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(feature = "ts", ts(as = "Option<Vec<ReviewFinding>>", optional))]
        findings: Vec<ReviewFinding>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(feature = "ts", ts(as = "Option<Vec<ResolvedFinding>>", optional))]
        resolved: Vec<ResolvedFinding>,
    },
}

/// The control plane's answer to an [`AgentCall`].
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentAnswer {
    Memories {
        memories: Vec<Memory>,
    },
    /// The note as it is now: the new one, or the one marked stale. With
    /// `replaces`, also the note it replaced, and with `also_replaces` the
    /// others.
    Memory {
        memory: Memory,
        #[cfg_attr(feature = "ts", ts(optional))]
        replaced: Option<Memory>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(feature = "ts", ts(as = "Option<Vec<Memory>>", optional))]
        also_replaced: Vec<Memory>,
    },
    /// The retrospective recorded, or as it is now updated.
    Retrospective {
        retrospective: Retrospective,
    },
    /// The session's retrospectives, newest first.
    Retrospectives {
        retrospectives: Vec<Retrospective>,
    },
    /// The review thread as it is now.
    Thread {
        thread: ReviewThread,
    },
    /// fctl's review threads on the pull request, oldest first.
    ReviewThreads {
        threads: Vec<FindingThread>,
    },
    /// What fctl posted.
    ReviewPosted {
        review: PostedReview,
    },
}

/// A review thread of a pull request, as GitHub has it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ReviewThread {
    /// GitHub's id (`PRRT_…`).
    pub id: String,
    pub resolved: bool,
    /// The pull request it is on.
    pub pull_request_url: String,
}

// ───────────────────────────── GitHub ─────────────────────────────

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct GithubInfo {
    pub configured: bool,
    pub app_slug: Option<String>,
    /// Where to install the app on more accounts.
    pub install_url: Option<String>,
    /// The accounts your workspaces work in.
    pub accounts: Vec<String>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct GithubRepo {
    pub owner: String,
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub default_branch: String,
    pub description: Option<String>,
    pub pushed_at: Option<String>,
    /// The project already linked to it.
    pub project_id: Option<ProjectId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GithubRepoList {
    pub repos: Vec<GithubRepo>,
}

// ───────────────────────────── Claude credentials ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CredentialStatus {
    Active,
    /// Expired or revoked; connect again.
    Invalid,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ClaudeCredentialInfo {
    pub id: CredentialId,
    pub kind: ClaudeCredentialKind,
    pub label: Option<String>,
    pub account_email: Option<String>,
    pub subscription_type: Option<String>,
    /// Whether sessions on this credential can enable Remote Control.
    pub remote_control: bool,
    pub status: CredentialStatus,
    /// Used for new sessions.
    pub is_default: bool,
    pub created_at: Millis,
    pub last_used_at: Option<Millis>,
    pub expires_at: Option<Millis>,
    /// How much of the plan's usage limits is used, as the sessions on this
    /// credential last reported it: one entry per limit, the five-hour
    /// window first. Empty until a session reports one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<PlanLimit>>", optional))]
    pub usage: Vec<PlanLimit>,
}

/// Where a Claude plan limit stands, as Claude reported it for the limit
/// that is closest to stopping requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum LimitStatus {
    Allowed,
    /// Close to the limit.
    AllowedWarning,
    /// The limit is reached: sessions pause until it resets.
    Rejected,
}

impl LimitStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::AllowedWarning => "allowed_warning",
            Self::Rejected => "rejected",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "allowed" => Some(Self::Allowed),
            "allowed_warning" => Some(Self::AllowedWarning),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

/// One of a Claude plan's usage limits, from the `rate_limit_event`s that
/// sessions report.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct PlanLimit {
    /// The limit's window, as Claude names it: `five_hour`, `seven_day`
    /// (the week, every model), or a model's own weekly limit
    /// (`seven_day_opus`, `seven_day_sonnet`, …).
    pub window: String,
    /// The share of the limit used, from 0 to 1. Usage can run a little past
    /// the limit, so it can be more.
    pub utilization: Option<f64>,
    /// When the window starts over.
    pub resets_at: Option<Millis>,
    /// Present when Claude reported this limit as the one closest to
    /// stopping requests.
    pub status: Option<LimitStatus>,
    /// When a session reported it.
    pub observed_at: Millis,
}

#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AddClaudeTokenRequest {
    /// The output of `claude setup-token` (`sk-ant-oat01-…`).
    pub token: String,
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ClaudeLoginState {
    /// Waiting for a runner to pick it up.
    Pending,
    Starting,
    /// Open `url`, approve, then submit the code claude.ai shows.
    AwaitingCode {
        url: String,
    },
    Verifying,
    Succeeded {
        credential_id: CredentialId,
    },
    Failed {
        error: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ClaudeLoginJob {
    pub id: LoginJobId,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(flatten))]
    pub state: ClaudeLoginState,
    pub created_at: Millis,
    pub updated_at: Millis,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ClaudeLoginCodeRequest {
    pub code: String,
}

// ───────────────────────────── notifications ─────────────────────────────

/// What notifies a person, in every browser where they turned notifications
/// on. A field that is missing has its default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct NotificationSettings {
    /// A session they follow finished its turn and waits for a reply.
    pub waiting: bool,
    /// A session they follow paused on a Claude usage limit.
    pub paused: bool,
    /// A session they follow failed.
    pub failed: bool,
    /// Starting or resuming a session follows it.
    pub follow_started: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            waiting: true,
            paused: true,
            failed: true,
            follow_started: true,
        }
    }
}

/// A browser that shows your notifications: its Web Push subscription.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct PushDevice {
    pub id: PushDeviceId,
    /// What the browser was called when it subscribed ("Chrome on macOS").
    pub name: Option<String>,
    /// The push service that delivers to it, e.g. `fcm.googleapis.com`.
    pub service: String,
    pub created_at: Millis,
    /// When a notification last went to it.
    pub last_sent_at: Option<Millis>,
    /// It subscribed with the sign-in this request uses: this browser.
    pub current: bool,
}

/// `GET /api/v1/me/notifications`.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct Notifications {
    pub settings: NotificationSettings,
    pub devices: Vec<PushDevice>,
    /// The instance's VAPID public key (base64url), for
    /// `PushManager.subscribe`. Absent when it cannot send notifications.
    pub public_key: Option<String>,
}

/// `PATCH /api/v1/me/notifications`: the settings to change.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateNotificationsRequest {
    pub waiting: Option<bool>,
    pub paused: Option<bool>,
    pub failed: Option<bool>,
    pub follow_started: Option<bool>,
}

/// A browser's keys for encrypting notifications to it, from
/// `PushSubscription.toJSON()`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PushKeys {
    /// Its P-256 public key, base64url.
    pub p256dh: String,
    /// Its authentication secret, base64url.
    pub auth: String,
}

/// `POST /api/v1/me/notifications/devices`: notifications for a browser.
/// Sending the same endpoint again updates it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct AddPushDeviceRequest {
    /// `PushSubscription.endpoint`.
    pub endpoint: String,
    pub keys: PushKeys,
    /// What to call it, e.g. "Chrome on macOS".
    pub name: Option<String>,
}

/// `POST /api/v1/me/notifications/test`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PushTestResult {
    /// Browsers whose push service took the notification.
    pub sent: u32,
    /// Browsers it could not reach. Expired ones are removed.
    pub failed: u32,
}

/// A notification as the control plane sends it to a browser (encrypted,
/// RFC 8291), for its service worker (`web/public/sw.js`) to show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PushMessage {
    pub title: String,
    pub body: String,
    /// Replaces a notification with the same tag that is still shown.
    pub tag: String,
    /// The page a click opens, a path in the app.
    pub url: String,
    /// Tags of the notifications this one sums up: showing it closes them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
    pub replaces: Vec<String>,
    /// When it happened.
    pub at: Millis,
}

// ───────────────────────────── feed (dashboard live updates) ─────────────────────────────

/// `GET /api/v1/feed/ws`: live updates for the dashboard.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FeedServer {
    Hello {
        runners: Vec<Runner>,
        server_time: Millis,
    },
    /// A session the person can see changed.
    Session {
        session: Box<Session>,
    },
    /// A session was deleted or the person lost access.
    SessionRemoved {
        session_id: SessionId,
    },
    Runner {
        runner: Box<Runner>,
    },
    /// Agent activity for the visualization: one pulse per event, by session.
    Activity {
        session_id: SessionId,
        project_id: ProjectId,
        /// Event kind or Claude message type (`assistant`, `tool_use`, …).
        kind: String,
        /// Rough size, for the visual weight of the pulse.
        weight: u32,
    },
    ClaudeLogin {
        job: Box<ClaudeLoginJob>,
    },
    /// A session reported new plan usage for one of the person's Claude
    /// credentials: all its limits, as `ClaudeCredentialInfo.usage`.
    ClaudeUsage {
        credential_id: CredentialId,
        usage: Vec<PlanLimit>,
    },
    /// The person's workspaces or invitations changed (they joined or left
    /// one, were invited, or an admin changed one): fetch them again.
    Workspaces,
    Pong,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FeedClient {
    Ping,
}

/// `{}` responses.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Empty {}

/// `{"ok": true}` responses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Ok {
    pub ok: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_settings_default_to_everything() -> Result<(), serde_json::Error> {
        let stored: NotificationSettings = serde_json::from_str(r#"{"waiting":false}"#)?;
        assert_eq!(
            stored,
            NotificationSettings {
                waiting: false,
                ..Default::default()
            }
        );
        assert!(stored.paused && stored.failed && stored.follow_started);
        Ok(())
    }

    #[test]
    fn runners_from_older_control_planes_have_no_drain() -> Result<(), serde_json::Error> {
        let old: Runner = serde_json::from_value(serde_json::json!({
            "id": "rnr_1", "name": "studio", "status": "online", "drivers": [],
            "capacity": 4, "active_sessions": 1, "labels": [], "created_at": 1
        }))?;
        assert_eq!(old.drain, None);
        // An update from an older client changes nothing about draining.
        let update: UpdateRunnerRequest = serde_json::from_str(r#"{"enabled":true}"#)?;
        assert_eq!(update.draining, None);
        Ok(())
    }

    #[test]
    fn workspaces_from_older_control_planes_ask_for_invites() -> Result<(), serde_json::Error> {
        let old: Workspace = serde_json::from_value(serde_json::json!({
            "id": "ws_1", "slug": "side-project", "name": "Side project", "color": "pink",
            "role": "owner", "email": "ada@example.com", "base_role": "editor",
            "require_two_factor": false, "blocked": false, "members": 1, "projects": 0,
            "runners": 0, "runners_online": 0, "github": [], "created_at": 1
        }))?;
        assert!(!old.skip_invites);
        // An update from an older client leaves the step as it is.
        let update: UpdateWorkspaceRequest = serde_json::from_str(r#"{"name":"Side"}"#)?;
        assert_eq!(update.skip_invites, None);
        assert!(!serde_json::to_string(&update)?.contains("skip_invites"));
        Ok(())
    }

    #[test]
    fn push_messages_leave_out_what_they_do_not_replace() -> Result<(), serde_json::Error> {
        let m = PushMessage {
            title: "Fix the login test".into(),
            body: "my-app#12 waits for your reply".into(),
            tag: "ses_1".into(),
            url: "/s/ses_1".into(),
            replaces: vec![],
            at: 1,
        };
        let json = serde_json::to_string(&m)?;
        assert!(!json.contains("replaces"));
        assert_eq!(serde_json::from_str::<PushMessage>(&json)?, m);
        Ok(())
    }
}

#[cfg(test)]
mod two_factor_tests {
    use super::ChallengeReason;

    #[test]
    fn challenge_reasons_round_trip_as_serde_names_them() {
        for r in [
            ChallengeReason::Password,
            ChallengeReason::Google,
            ChallengeReason::PasswordReset,
        ] {
            let json = serde_json::to_string(&r).unwrap_or_default();
            assert_eq!(json, format!("\"{}\"", r.as_str()));
            assert_eq!(ChallengeReason::parse(r.as_str()), Some(r));
        }
        assert_eq!(ChallengeReason::parse("phone"), None);
    }
}
