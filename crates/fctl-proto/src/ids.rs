//! Typed identifiers. Every id is a string with a type prefix, for example
//! `usr_01JABCDEF…`, so ids are self-describing in logs, URLs and tools.
//! Generation lives in `fctl-domain`.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// The prefix every id of this type starts with (without the `_`).
            pub const PREFIX: &'static str = $prefix;

            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Whether `s` looks like an id of this type (prefix and charset only).
            pub fn is_valid(s: &str) -> bool {
                s.len() > Self::PREFIX.len() + 1
                    && s.len() <= 64
                    && s.starts_with(Self::PREFIX)
                    && s.as_bytes().get(Self::PREFIX.len()) == Some(&b'_')
                    && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            }

            /// Parses an id, checking its prefix.
            pub fn parse(s: &str) -> Option<Self> {
                Self::is_valid(s).then(|| Self(s.to_owned()))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

id_type!(
    /// A person with access to the instance.
    UserId,
    "usr"
);
id_type!(
    /// A project: a repository plus the settings agents use for it.
    ProjectId,
    "prj"
);
id_type!(
    /// One agent session (one Claude conversation in one sandbox).
    SessionId,
    "ses"
);
id_type!(
    /// A machine that runs sandboxes (a host).
    RunnerId,
    "rnr"
);
id_type!(
    /// A stored Claude credential.
    CredentialId,
    "crd"
);
id_type!(
    /// An allowlist entry.
    AllowId,
    "alw"
);
id_type!(
    /// A web session, CLI token, personal access token or runner token.
    TokenId,
    "tok"
);
id_type!(
    /// An invitation link.
    InviteId,
    "inv"
);
id_type!(
    /// A message, interrupt or other delivery to an agent.
    DeliveryId,
    "dlv"
);
id_type!(
    /// A Claude sign-in job run on a host.
    LoginJobId,
    "lgn"
);
id_type!(
    /// A linked external identity (Google).
    IdentityId,
    "idn"
);
id_type!(
    /// A file someone attached to a message.
    AttachmentId,
    "att"
);
id_type!(
    /// A browser that shows someone's notifications (a Web Push subscription).
    PushDeviceId,
    "psh"
);
id_type!(
    /// A passkey someone signs in with.
    PasskeyId,
    "pky"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_prefix_and_charset() {
        assert!(UserId::is_valid("usr_01JABCDEFGHJKMNPQRSTVWXYZ0"));
        assert!(!UserId::is_valid("prj_01JABCDEFGHJKMNPQRSTVWXYZ0"));
        assert!(!UserId::is_valid("usr_"));
        assert!(!UserId::is_valid("usr-01JABC"));
        assert!(!UserId::is_valid("usr_01J/../x"));
        assert_eq!(
            SessionId::parse("ses_abc").map(|s| s.0),
            Some("ses_abc".into())
        );
    }

    #[test]
    fn serializes_transparently() {
        let id = ProjectId("prj_x1".into());
        assert_eq!(serde_json::to_string(&id).ok(), Some("\"prj_x1\"".into()));
    }
}
