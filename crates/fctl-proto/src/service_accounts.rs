//! Service accounts (docs/architecture.md#service-accounts): a workspace's
//! credential for a program of its own (a portal where people start
//! agents, a chat bot, a script) that starts sessions for the workspace's
//! people, runs its schedules and reads its sessions, through the REST API
//! and the CLI.
//!
//! A service account acts for one of the workspace's people at a time,
//! named in each request (`Fctl-On-Behalf-Of`, `fctl --on-behalf-of`): the session
//! it starts is theirs, on their Claude subscription, within what they may
//! do in the project. Where the instance asks people first
//! ([`crate::api::InstanceSettings::service_account_consent`], on unless
//! its admins turn it off), it acts only for those who allowed it.

use crate::Millis;
use crate::ServiceAccountId;
use crate::session::{UserRef, WorkspaceRef, WorkspaceRole};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// The header a service account names the person it acts for in, by their
/// id (`usr_…`) or one of their addresses.
pub const ON_BEHALF_OF: &str = "Fctl-On-Behalf-Of";

/// How a person answered a service account's ask to start sessions for
/// them. Absent while they haven't answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ServiceAccountAnswer {
    /// It may start sessions for them, on their subscription.
    Allowed,
    /// It may not.
    Declined,
    /// One a newer control plane knows, which this version doesn't.
    #[serde(other)]
    Other,
}

impl ServiceAccountAnswer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Declined => "declined",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "allowed" => Some(Self::Allowed),
            "declined" => Some(Self::Declined),
            _ => None,
        }
    }
}

/// How its people answered a service account, in numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServiceAccountTally {
    pub allowed: u32,
    pub declined: u32,
    /// Members who haven't answered yet.
    pub unanswered: u32,
}

/// A workspace's service account. Every member sees its name, what it is
/// and their own answer; its admins see the rest.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ServiceAccount {
    pub id: ServiceAccountId,
    pub workspace: WorkspaceRef,
    pub name: String,
    /// What it is and where people use it, in a sentence or two, for the
    /// people it asks: "Our agent portal at portal.acme.dev".
    pub description: Option<String>,
    pub created_by: Option<UserRef>,
    pub created_at: Millis,
    /// Your answer to it. Absent while you haven't answered.
    pub answer: Option<ServiceAccountAnswer>,
    pub answered_at: Option<Millis>,
    /// Whether it may start sessions for you now: you allowed it, or the
    /// instance doesn't ask people (`consent` false).
    pub acts_for_you: bool,
    /// Whether people choose: where the instance asks them first, it acts
    /// only for those who allowed it; where it doesn't, for everyone.
    pub consent: bool,
    /// The admin who made its secret: it works while they are an active
    /// admin of the workspace (its admins).
    pub keeper: Option<UserRef>,
    /// Why it works for nobody now, when it doesn't: its secret's maker
    /// is no longer an admin here. A new secret starts it again (its
    /// admins).
    pub stopped: Option<String>,
    /// The end of its secret, to tell secrets apart: `x8Kq` (its admins).
    pub secret_hint: Option<String>,
    /// When its secret was made: as it was made, or since replaced.
    pub secret_made_at: Option<Millis>,
    /// When it last made a request (its admins).
    pub last_used_at: Option<Millis>,
    /// How its people answered (its admins).
    pub people: Option<ServiceAccountTally>,
    /// The sessions it started, in all (its admins).
    pub sessions: Option<u32>,
    /// You may change it, make it a new secret and delete it: you are one
    /// of the workspace's admins.
    pub can_manage: bool,
}

/// A service account in brief, as an invitation to its workspace shows
/// it.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ServiceAccountRef {
    pub id: ServiceAccountId,
    pub name: String,
    pub description: Option<String>,
}

/// `GET /api/v1/workspaces/{id}/service-accounts`: the workspace's, in the
/// order they were made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServiceAccountList {
    pub service_accounts: Vec<ServiceAccount>,
    /// Whether people choose ([`ServiceAccount::consent`]).
    pub consent: bool,
    /// The most a workspace may have.
    pub limit: u32,
}

/// `POST /api/v1/workspaces/{id}/service-accounts` (its admins).
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct CreateServiceAccountRequest {
    pub name: String,
    pub description: Option<String>,
}

/// A new service account, or one with a new secret, with the secret,
/// shown once.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreatedServiceAccount {
    pub service_account: ServiceAccount,
    pub secret: String,
}

/// `PATCH /api/v1/service-accounts/{id}` (its workspace's admins): absent
/// fields stay as they are; an empty description removes it.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct UpdateServiceAccountRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// `PUT /api/v1/service-accounts/{id}/answer`: whether it may start
/// sessions for you. You change it whenever you like.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AnswerServiceAccountRequest {
    pub allow: bool,
}

/// One of the workspace's people, as a service account sees them: whether
/// it may act for them, and why not.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ServiceAccountPerson {
    pub user: UserRef,
    /// The address they are a member as.
    pub email: String,
    pub role: WorkspaceRole,
    pub answer: Option<ServiceAccountAnswer>,
    pub answered_at: Option<Millis>,
    /// Whether it may act for them now.
    pub acts_for: bool,
    /// It can't act for them whatever they answer: their account is
    /// disabled, or they have to set up two-factor sign-in first.
    #[serde(default)]
    pub blocked: bool,
    /// Why it may not, when it may not.
    pub why_not: Option<String>,
}

/// `GET /api/v1/service-accounts/{id}/people` (its workspace's admins, and
/// the service account itself): the workspace's people, by name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServiceAccountPeople {
    pub people: Vec<ServiceAccountPerson>,
    /// Whether people choose ([`ServiceAccount::consent`]).
    pub consent: bool,
}

/// `GET /api/v1/service-account`, with a service account's token: what it
/// is, and whom it acts for in this request, when it names someone.
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(optional_fields))]
pub struct ServiceAccountSelf {
    pub service_account: ServiceAccount,
    /// The person it acts for in this request ([`ON_BEHALF_OF`]).
    pub acting_for: Option<UserRef>,
    /// The address they are a member of its workspace as.
    pub acting_for_email: Option<String>,
}
