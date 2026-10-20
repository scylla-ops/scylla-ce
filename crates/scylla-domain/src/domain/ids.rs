use std::fmt;

#[must_use]
pub fn new_id() -> String {
    ulid::Ulid::generate().to_string().to_lowercase()
}

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, ::serde::Serialize, ::serde::Deserialize)]
        pub struct $name(String);

        impl $name {
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            pub fn generate() -> Self {
                Self($crate::domain::ids::new_id())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self::new(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self::new(s)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

define_id!(UserId);
define_id!(OrganizationId);
define_id!(ProjectId);
define_id!(PipelineId);
define_id!(JobId);
define_id!(JobLogId);
define_id!(SessionId);
define_id!(AppId);
define_id!(AppTokenId);
define_id!(AppCredentialId);
define_id!(SecretId);
define_id!(TriggerId);
define_id!(GrantId);
define_id!(StreamId);
define_id!(PasswordResetId);
