//! The API tokens: long secrets that a program exchanges for short tokens of their actor. The
//! server keeps only a hash of each secret.

use time::{Duration, OffsetDateTime};

use crate::identity::actors::{Actor, ActorId};
use crate::projects::ProjectId;
use crate::{DomainError, Id, Name};

/// The start of the secret of an API token of an App.
pub const APP_TOKEN_PREFIX: &str = "scya_";

/// The start of the secret of a personal token of a user.
pub const PERSONAL_TOKEN_PREFIX: &str = "scyp_";

/// The number of last characters of a secret that its hint shows.
pub const HINT_LEN: usize = 4;

/// The marker of an API token id.
pub enum ApiToken {}

/// The id of an API token.
pub type ApiTokenId = Id<ApiToken>;

/// The name of an API token.
pub type TokenName = Name;

/// The life of a new API token. A token always expires.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TokenExpiry {
    /// 7 days.
    Days7,
    /// 30 days.
    Days30,
    /// 90 days.
    #[default]
    Days90,
    /// 365 days.
    Days365,
}

impl TokenExpiry {
    /// Returns the life of a token.
    pub fn duration(self) -> Duration {
        Duration::days(match self {
            Self::Days7 => 7,
            Self::Days30 => 30,
            Self::Days90 => 90,
            Self::Days365 => 365,
        })
    }
}

/// An API token, as the store returns it. It has no secret.
#[derive(Clone, Debug)]
pub struct ApiTokenInfo {
    /// The id.
    pub id: ApiTokenId,
    /// The actor of the token.
    pub actor_id: ActorId,
    /// The name.
    pub name: String,
    /// The prefix and the last characters of the secret.
    pub hint: String,
    /// The actor that created the token.
    pub created_by: ActorId,
    /// The creation time.
    pub created_at: OffsetDateTime,
    /// The end of its validity.
    pub expires_at: OffsetDateTime,
    /// The last exchange of the token.
    pub last_used_at: Option<OffsetDateTime>,
    /// The time of its revocation.
    pub revoked_at: Option<OffsetDateTime>,
    /// What its short tokens can do.
    pub limits: TokenLimits,
}

/// The starts of the names of the read-only methods.
pub const READ_PREFIXES: [&str; 4] = ["Get", "List", "Check", "Tail"];

/// The methods that the deploy preset adds to the read-only methods.
pub const DEPLOY_METHODS: [&str; 4] = [
    "scylla.pipelines.v1.PipelineService/RunPipeline",
    "scylla.pipelines.v1.TriggerService/FireTriggerNow",
    "scylla.pipelines.v1.JobService/CancelJob",
    "scylla.pipelines.v1.JobService/RerunJob",
];

/// The largest number of methods of a custom preset.
pub const MAX_ACTIONS: usize = 200;

/// The largest number of projects of a token.
pub const MAX_PROJECTS: usize = 100;

/// A set of methods for an API token.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TokenPreset {
    /// The methods whose names start with one of `READ_PREFIXES`.
    #[default]
    ReadOnly,
    /// The read-only methods and `DEPLOY_METHODS`.
    Deploy,
    /// Each method that the roles of the actor allow.
    Full,
    /// The methods of the list of the token only.
    Custom,
}

impl TokenPreset {
    /// Returns the name of the preset in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Deploy => "deploy",
            Self::Full => "full",
            Self::Custom => "custom",
        }
    }

    /// Reads a name of `as_str`. Another name gives the read-only preset.
    pub fn from_name(name: &str) -> Self {
        match name {
            "deploy" => Self::Deploy,
            "full" => Self::Full,
            "custom" => Self::Custom,
            _ => Self::ReadOnly,
        }
    }
}

/// What the short tokens of an API token can do, inside the rights of its actor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TokenLimits {
    /// The set of methods.
    pub preset: TokenPreset,
    /// The methods of a custom preset, sorted, with no duplicate.
    pub actions: Vec<String>,
    /// The projects that the token can reach, sorted, with no duplicate. Empty: no limit.
    pub projects: Vec<ProjectId>,
}

impl TokenLimits {
    /// Checks limits, and returns the field and the error of a bad value.
    pub fn new(
        preset: TokenPreset,
        mut actions: Vec<String>,
        mut projects: Vec<ProjectId>,
    ) -> Result<Self, (&'static str, DomainError)> {
        match (preset, actions.is_empty()) {
            (TokenPreset::Custom, true) => return Err(("actions", DomainError::Empty)),
            (TokenPreset::Custom, false) | (_, true) => {}
            (_, false) => return Err(("actions", DomainError::NotCustom)),
        }
        if actions.len() > MAX_ACTIONS {
            return Err(("actions", DomainError::TooMany { max: MAX_ACTIONS }));
        }
        if !actions.iter().all(|action| is_method(action)) {
            return Err(("actions", DomainError::NotAMethod));
        }
        if projects.len() > MAX_PROJECTS {
            return Err(("project_ids", DomainError::TooMany { max: MAX_PROJECTS }));
        }
        actions.sort();
        actions.dedup();
        projects.sort_by_key(|id| id.to_db());
        projects.dedup();
        Ok(Self {
            preset,
            actions,
            projects,
        })
    }
}

/// Returns `true` for the name of a gRPC method: `<package>.<Service>/<Method>`.
fn is_method(text: &str) -> bool {
    let Some((service, method)) = text.split_once('/') else {
        return false;
    };
    let Some((package, service)) = service.rsplit_once('.') else {
        return false;
    };
    let lower = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_';
    let pascal = |name: &str| {
        name.starts_with(|c: char| c.is_ascii_uppercase())
            && name.chars().all(|c| c.is_ascii_alphanumeric())
    };
    !package.is_empty() && package.chars().all(lower) && pascal(service) && pascal(method)
}

/// Returns the hint of a secret: its prefix, `...`, and its last characters.
pub fn hint(prefix: &str, secret: &str) -> String {
    let start = secret.len().saturating_sub(HINT_LEN);
    let end = secret.get(start..).unwrap_or_default();
    format!("{prefix}...{end}")
}

crate::authz! {
    /// The routes of a user on its own personal tokens.
    extend Actor as TokenOwner {
        /// The route `CreatePersonalToken`.
        permission create_personal_token = nil;
        /// The route `ListPersonalTokens`.
        permission list_personal_tokens = nil;
        /// The route `RevokePersonalToken`.
        permission revoke_personal_token = nil;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hint_shows_the_prefix_and_the_end_only() {
        assert_eq!(hint(APP_TOKEN_PREFIX, "scya_abcdefgh"), "scya_...efgh");
    }

    #[test]
    fn the_limits_check_their_actions_and_their_projects() {
        let run = "scylla.pipelines.v1.PipelineService/RunPipeline".to_owned();
        let custom = TokenLimits::new(TokenPreset::Custom, vec![run.clone(), run.clone()], vec![]);
        assert_eq!(custom.unwrap().actions, vec![run.clone()]);
        let empty = TokenLimits::new(TokenPreset::Custom, vec![], vec![]);
        assert_eq!(empty, Err(("actions", DomainError::Empty)));
        let not_custom = TokenLimits::new(TokenPreset::Deploy, vec![run], vec![]);
        assert_eq!(not_custom, Err(("actions", DomainError::NotCustom)));
        for bad in [
            "RunPipeline",
            "Pipeline/Run",
            "scylla.v1.pipelineService/Run",
            "a.B/c",
        ] {
            let limits = TokenLimits::new(TokenPreset::Custom, vec![bad.to_owned()], vec![]);
            assert_eq!(limits, Err(("actions", DomainError::NotAMethod)), "{bad}");
        }
    }

    #[test]
    fn the_default_life_is_90_days_and_the_longest_is_one_year() {
        assert_eq!(TokenExpiry::default().duration(), Duration::days(90));
        assert_eq!(TokenExpiry::Days365.duration(), Duration::days(365));
    }
}
