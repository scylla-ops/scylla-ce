//! The secrets of a project: named values that the runs of its pipelines get. No answer of the
//! API holds a value.

use std::fmt;

use time::OffsetDateTime;

use crate::identity::actors::ActorId;
use crate::projects::ProjectId;
use crate::{DomainError, Id, Page, PageRequest};

/// The id of a secret.
pub type SecretId = Id<Secret>;

/// The name of a secret: 1 to `MAX_LEN` ASCII letters, digits, dots, dashes or underscores,
/// unique in its project. A pipeline names a secret by its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretName(String);

impl SecretName {
    /// The maximum number of characters.
    pub const MAX_LEN: usize = 128;

    /// Trims the text and checks its form. The case stays.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        let name = raw.trim();
        let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_');
        let valid = (1..=Self::MAX_LEN).contains(&name.len()) && name.chars().all(allowed);
        if !valid {
            return Err(DomainError::NotASecretName);
        }
        Ok(Self(name.to_owned()))
    }

    /// Returns the name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The value of a secret: at most `MAX_BYTES` bytes, kept as it is. Its text never goes to a
/// log: `Debug` shows `***`, and it has no `Display`.
#[derive(Clone)]
pub struct SecretValue(String);

impl SecretValue {
    /// The maximum number of bytes, the limit of the value of an environment variable of a run.
    pub const MAX_BYTES: usize = 65_536;

    /// Checks the size. The text is kept as it is: spaces and line breaks count.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        if raw.len() > Self::MAX_BYTES {
            return Err(DomainError::TooLarge {
                max: Self::MAX_BYTES,
            });
        }
        Ok(Self(raw))
    }

    /// Returns the text, for the encryption and for the run that uses it only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(***)")
    }
}

/// A secret, as the store returns it: its metadata, never its value.
#[derive(Clone, Debug)]
pub struct Secret {
    /// The id of the secret.
    pub id: SecretId,
    /// The project that holds the secret.
    pub project_id: ProjectId,
    /// The name, unique in the project.
    pub name: SecretName,
    /// The actor that created the secret.
    pub created_by: ActorId,
    /// The creation time, set by the database.
    pub created_at: OffsetDateTime,
    /// The time of the last change of the value, set by the database.
    pub updated_at: OffsetDateTime,
}

/// The input of a secret creation.
#[derive(Clone, Debug)]
pub struct NewSecret {
    /// The project of the new secret.
    pub project_id: ProjectId,
    /// The actor that creates the secret.
    pub created_by: ActorId,
    /// The name, unique in the project.
    pub name: SecretName,
    /// The value, in clear: the service encrypts it.
    pub value: SecretValue,
}

/// The input of a secret list: the secrets of one project, in name order.
#[derive(Clone, Debug)]
pub struct ListSecrets {
    /// The project.
    pub project_id: ProjectId,
    /// The page to return: the names come after the name of the token.
    pub page: PageRequest<SecretName>,
}

/// One page of secrets, in name order.
pub type SecretPage = Page<Secret, SecretName>;

#[cfg(test)]
mod tests {
    //! Unit tests of the names and of the values of the secrets.

    use super::{SecretName, SecretValue};
    use crate::DomainError;

    /// A name has 1 to 128 ASCII letters, digits, dots, dashes or underscores, in any case.
    #[test]
    fn parse_checks_the_name() {
        // given
        let (longest, too_long) = ("x".repeat(128), "x".repeat(129));
        let valid = ["A", "db.PASSWORD-2_x", " TOKEN ", longest.as_str()];
        let invalid = ["", "  ", "a b", "a/b", "é", "a$", too_long.as_str()];

        // when
        let parsed: Vec<_> = valid.map(|raw| SecretName::parse(raw.to_owned())).into();
        let refused: Vec<_> = invalid.map(|raw| SecretName::parse(raw.to_owned())).into();

        // then
        assert!(parsed.iter().all(Result::is_ok), "{parsed:?}");
        assert_eq!(parsed[2].as_ref().map(SecretName::as_str), Ok("TOKEN"));
        assert!(refused
            .iter()
            .all(|r| r == &Err(DomainError::NotASecretName)));
    }

    /// A value of 65536 bytes passes, one more byte fails, and the text stays as it is.
    #[test]
    fn parse_checks_the_size_of_the_value() {
        // given
        let (largest, too_large) = ("é".repeat(32_768), format!("{}x", "é".repeat(32_768)));

        // when
        let kept = SecretValue::parse(" a\nb ".to_owned());
        let largest = SecretValue::parse(largest);
        let too_large = SecretValue::parse(too_large);

        // then
        assert_eq!(
            kept.map(|value| value.expose().to_owned()),
            Ok(" a\nb ".to_owned())
        );
        assert!(largest.is_ok());
        assert!(matches!(
            too_large,
            Err(DomainError::TooLarge { max: 65_536 })
        ));
    }

    /// The debug text of a value hides it.
    #[test]
    fn debug_hides_the_value() {
        // given
        let value = SecretValue::parse("hunter2-value".to_owned()).unwrap();

        // when
        let shown = format!("{value:?} {:?}", Some(&value));

        // then
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("***"));
    }
}
