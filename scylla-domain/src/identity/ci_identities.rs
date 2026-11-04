//! The CI identities of the Apps: the jobs of a repository of a CI provider get a token of an
//! App with the ID token of their job, with no secret.

use time::OffsetDateTime;

use crate::identity::actors::ActorId;
use crate::identity::api_tokens::TokenLimits;
use crate::identity::apps::AppId;
use crate::{DomainError, Id};

/// The issuer of the ID tokens of GitHub Actions.
pub const GITHUB_ISSUER: &str = "https://token.actions.githubusercontent.com";

/// The issuer of the ID tokens of GitLab.com.
pub const GITLAB_ISSUER: &str = "https://gitlab.com";

/// The longest text of a repository, a ref, an environment or an issuer.
pub const MAX_LEN: usize = 255;

/// The marker of a CI identity id.
pub enum CiIdentity {}

/// The id of a CI identity.
pub type CiIdentityId = Id<CiIdentity>;

/// A CI provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CiProvider {
    /// GitHub Actions.
    GitHub,
    /// GitLab CI.
    GitLab,
}

impl CiProvider {
    /// Returns the name of the provider in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::GitLab => "gitlab",
        }
    }

    /// Reads a name of `as_str`. Another name gives GitHub.
    pub fn from_name(name: &str) -> Self {
        match name {
            "gitlab" => Self::GitLab,
            _ => Self::GitHub,
        }
    }

    /// Returns the issuer of the public service of the provider.
    pub fn default_issuer(self) -> &'static str {
        match self {
            Self::GitHub => GITHUB_ISSUER,
            Self::GitLab => GITLAB_ISSUER,
        }
    }

    /// Returns the claim of the ID token that names the repository.
    pub fn repository_claim(self) -> &'static str {
        match self {
            Self::GitHub => "repository",
            Self::GitLab => "project_path",
        }
    }
}

/// The claims of the ID token of a CI job that a CI identity checks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CiClaims {
    /// The repository of the job.
    pub repository: Option<String>,
    /// The Git ref of the job.
    pub git_ref: Option<String>,
    /// The environment of the job.
    pub environment: Option<String>,
}

/// A new CI identity, checked.
#[derive(Clone, Debug)]
pub struct NewCiIdentity {
    /// The CI provider.
    pub provider: CiProvider,
    /// The issuer, with no `/` at the end.
    pub issuer: String,
    /// The repository.
    pub repository: String,
    /// The Git ref, or `None` for each ref.
    pub git_ref: Option<String>,
    /// The environment, or `None` for each environment.
    pub environment: Option<String>,
    /// What the tokens of the CI jobs can do.
    pub limits: TokenLimits,
}

impl NewCiIdentity {
    /// Checks a new CI identity. An empty issuer gives the issuer of the public service of the
    /// provider. Returns the field and the error of a bad value.
    pub fn new(
        provider: CiProvider,
        issuer: &str,
        repository: &str,
        git_ref: &str,
        environment: &str,
        limits: TokenLimits,
    ) -> Result<Self, (&'static str, DomainError)> {
        let issuer = match issuer.trim() {
            "" => provider.default_issuer(),
            issuer => issuer.trim_end_matches('/'),
        };
        if !(issuer.starts_with("https://") || issuer.starts_with("http://")) {
            return Err(("issuer", DomainError::NotAnUrl));
        }
        let repository = repository.trim();
        if !repository.contains('/') {
            return Err(("repository", DomainError::NotARepository));
        }
        Ok(Self {
            provider,
            issuer: text(issuer, "issuer")?,
            repository: text(repository, "repository")?,
            git_ref: optional(git_ref, "git_ref")?,
            environment: optional(environment, "environment")?,
            limits,
        })
    }
}

/// A CI identity, as the store returns it.
#[derive(Clone, Debug)]
pub struct CiIdentityInfo {
    /// The id.
    pub id: CiIdentityId,
    /// The App.
    pub app_id: AppId,
    /// The CI provider.
    pub provider: CiProvider,
    /// The issuer.
    pub issuer: String,
    /// The repository.
    pub repository: String,
    /// The Git ref, or `None` for each ref.
    pub git_ref: Option<String>,
    /// The environment, or `None` for each environment.
    pub environment: Option<String>,
    /// What the tokens of the CI jobs can do.
    pub limits: TokenLimits,
    /// The actor that added it.
    pub created_by: ActorId,
    /// The time when it was added.
    pub created_at: OffsetDateTime,
    /// The last exchange.
    pub last_used_at: Option<OffsetDateTime>,
}

impl CiIdentityInfo {
    /// Returns `true` if the claims of a job fit: the same repository, and the same ref and
    /// environment when the identity names them.
    pub fn matches(&self, claims: &CiClaims) -> bool {
        let fits = |rule: &Option<String>, value: &Option<String>| {
            rule.as_ref()
                .is_none_or(|rule| value.as_ref() == Some(rule))
        };
        claims.repository.as_deref() == Some(self.repository.as_str())
            && fits(&self.git_ref, &claims.git_ref)
            && fits(&self.environment, &claims.environment)
    }
}

/// Checks a text of one line and at most `MAX_LEN` characters.
fn text(value: &str, field: &'static str) -> Result<String, (&'static str, DomainError)> {
    if value.chars().count() > MAX_LEN {
        return Err((field, DomainError::TooLong { max: MAX_LEN }));
    }
    if value.chars().any(char::is_control) {
        return Err((field, DomainError::NotOneLine));
    }
    Ok(value.to_owned())
}

/// Checks an optional text: an empty text gives `None`.
fn optional(
    value: &str,
    field: &'static str,
) -> Result<Option<String>, (&'static str, DomainError)> {
    match value.trim() {
        "" => Ok(None),
        value => text(value, field).map(Some),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Makes a CI identity of a repository with a ref rule.
    fn identity(git_ref: Option<&str>) -> CiIdentityInfo {
        CiIdentityInfo {
            id: CiIdentityId::generate(),
            app_id: AppId::generate(),
            provider: CiProvider::GitHub,
            issuer: GITHUB_ISSUER.to_owned(),
            repository: "acme/web".to_owned(),
            git_ref: git_ref.map(str::to_owned),
            environment: None,
            limits: TokenLimits::default(),
            created_by: ActorId::generate(),
            created_at: OffsetDateTime::now_utc(),
            last_used_at: None,
        }
    }

    #[test]
    fn a_rule_must_fit_and_no_rule_takes_each_value() {
        let job = |repository: &str, git_ref: &str| CiClaims {
            repository: Some(repository.to_owned()),
            git_ref: Some(git_ref.to_owned()),
            environment: Some("prod".to_owned()),
        };

        assert!(identity(None).matches(&job("acme/web", "refs/heads/dev")));
        assert!(identity(Some("refs/heads/main")).matches(&job("acme/web", "refs/heads/main")));
        assert!(!identity(Some("refs/heads/main")).matches(&job("acme/web", "refs/heads/dev")));
        assert!(!identity(None).matches(&job("acme/api", "refs/heads/main")));
    }

    #[test]
    fn a_new_identity_checks_its_issuer_and_its_repository() {
        let limits = TokenLimits::default;
        let new = |issuer: &str, repository: &str| {
            NewCiIdentity::new(CiProvider::GitLab, issuer, repository, "", "", limits())
        };

        assert_eq!(new("", "group/project").unwrap().issuer, GITLAB_ISSUER);
        assert_eq!(
            new("ftp://x", "a/b").unwrap_err(),
            ("issuer", DomainError::NotAnUrl)
        );
        assert_eq!(
            new("", "project").unwrap_err(),
            ("repository", DomainError::NotARepository)
        );
    }
}
