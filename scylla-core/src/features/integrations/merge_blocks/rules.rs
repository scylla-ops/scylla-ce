//! The pure rules of the merge blocks: the name of the report of a pipeline, the body of the
//! GitHub ruleset, the state of a refusal, and the state that a list finds on the provider.

use std::collections::BTreeSet;

use scylla_domain::integrations::merge_blocks::MergeBlockState;
use scylla_domain::integrations::reports::{pipeline_key, report_context};
use scylla_domain::integrations::VcsProvider;
use serde_json::{json, Value};

use crate::infra::vcs::ProviderError;

/// The name of the one ruleset of Scylla in a GitHub repository.
pub(super) const RULESET_NAME: &str = "Scylla required checks";

/// The branch condition of the ruleset: the default branch of the repository.
pub(super) const DEFAULT_BRANCH: &str = "~DEFAULT_BRANCH";

/// The reason of a GitHub installation with no `administration: write`.
pub(super) const ADMINISTRATION_PERMISSION_MISSING: &str = "ADMINISTRATION_PERMISSION_MISSING";

/// The reason of a GitLab service account below Maintainer.
pub(super) const ROLE_TOO_LOW: &str = "ROLE_TOO_LOW";

/// The reason of a private repository where GitHub allows no rule (GitHub Free).
pub(super) const PRIVATE_REPOSITORY_NOT_ALLOWED: &str = "PRIVATE_REPOSITORY_NOT_ALLOWED";

/// A change that the provider did not make: the state of the rule and its reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Refusal {
    /// The state of the rule.
    pub(super) state: MergeBlockState,
    /// The reason, as a stable code.
    pub(super) reason: &'static str,
}

/// The rule that a list found on the provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Observed {
    /// Scylla did not read the provider, or the read failed.
    Unknown,
    /// The contexts that the ruleset of Scylla requires on the default branch. Empty when the
    /// ruleset is missing or not active.
    GitHub(BTreeSet<String>),
    /// `true` when "Pipelines must succeed" is on in the GitLab project.
    GitLab(bool),
}

/// The ruleset of Scylla, as GitHub gives it.
#[derive(Clone, Debug, Default)]
pub(super) struct Ruleset {
    /// `active`, `evaluate` or `disabled`.
    pub(super) enforcement: String,
    /// The branches of the ruleset.
    pub(super) include: Vec<String>,
    /// The contexts of its required status checks.
    pub(super) contexts: Vec<String>,
}

impl Refusal {
    /// Makes a refusal.
    pub(super) const fn new(state: MergeBlockState, reason: &'static str) -> Self {
        Self { state, reason }
    }
}

/// Returns the name of the report of a pipeline: `scylla/` and its file key, else the key of
/// its name.
pub(super) fn context(name: &str, file_key: Option<&str>) -> String {
    let key = file_key.map_or_else(|| pipeline_key(name), str::to_owned);
    report_context(&key)
}

/// Returns `true` when a change needs a call to the provider: a rule to turn on, or a rule to
/// turn off that the provider can have.
pub(super) fn needs_call(previous: Option<MergeBlockState>, enabled: bool) -> bool {
    enabled
        || matches!(
            previous,
            Some(
                MergeBlockState::On
                    | MergeBlockState::Applying
                    | MergeBlockState::OutOfSync
                    | MergeBlockState::Failed
            )
        )
}

/// Returns the refusal of a connection with no permission or no role for the rule.
pub(super) fn permission_missing(provider: VcsProvider) -> Refusal {
    let reason = match provider {
        VcsProvider::GitHub => ADMINISTRATION_PERMISSION_MISSING,
        VcsProvider::GitLab => ROLE_TOO_LOW,
    };
    Refusal::new(MergeBlockState::NeedsPermission, reason)
}

/// Returns the refusal of a failed call. A GitHub refusal on a private repository is the limit
/// of GitHub Free.
pub(super) fn refusal(provider: VcsProvider, private: bool, error: &ProviderError) -> Refusal {
    let failed = |reason| Refusal::new(MergeBlockState::Failed, reason);
    match error {
        ProviderError::Forbidden if provider == VcsProvider::GitHub && private => {
            Refusal::new(MergeBlockState::NotPossible, PRIVATE_REPOSITORY_NOT_ALLOWED)
        }
        ProviderError::Forbidden => permission_missing(provider),
        ProviderError::RateLimited(_) => failed("PROVIDER_LIMIT"),
        ProviderError::Unauthorized | ProviderError::NotFound => failed("PROVIDER_REFUSED"),
        ProviderError::Invalid(_) | ProviderError::Transient(_) => failed("PROVIDER_UNAVAILABLE"),
    }
}

/// Returns each context once, in the order of the list.
pub(super) fn unique(contexts: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    contexts
        .into_iter()
        .filter(|context| seen.insert(context.clone()))
        .collect()
}

/// Returns the body of the ruleset of Scylla: the default branch, and one required check for
/// each context, from the GitHub App only.
pub(super) fn ruleset_body(contexts: &[String], app_id: u64) -> Value {
    let checks: Vec<Value> = contexts
        .iter()
        .map(|context| json!({ "context": context, "integration_id": app_id }))
        .collect();
    json!({
        "name": RULESET_NAME,
        "target": "branch",
        "enforcement": "active",
        "conditions": { "ref_name": { "include": [DEFAULT_BRANCH], "exclude": [] } },
        "rules": [{
            "type": "required_status_checks",
            "parameters": {
                "strict_required_status_checks_policy": false,
                "required_status_checks": checks,
            },
        }],
    })
}

/// Returns the contexts that a ruleset requires on the default branch: none for no ruleset, or
/// for a ruleset that is not active.
pub(super) fn required(ruleset: Option<&Ruleset>) -> BTreeSet<String> {
    match ruleset {
        Some(ruleset)
            if ruleset.enforcement == "active"
                && ruleset
                    .include
                    .iter()
                    .any(|branch| branch == DEFAULT_BRANCH) =>
        {
            ruleset.contexts.iter().cloned().collect()
        }
        _ => BTreeSet::new(),
    }
}

/// Returns the state of a rule after the read of the provider: a rule that is on becomes out of
/// sync when its report has a new name, or when the provider does not have it.
pub(super) fn observed(
    state: MergeBlockState,
    enabled: bool,
    stored: Option<&str>,
    current: Option<&str>,
    found: &Observed,
) -> MergeBlockState {
    if state != MergeBlockState::On || !enabled {
        return state;
    }
    let in_sync = stored == current
        && match found {
            Observed::Unknown => true,
            Observed::GitHub(contexts) => current.is_some_and(|context| contexts.contains(context)),
            Observed::GitLab(on) => *on,
        };
    match in_sync {
        true => MergeBlockState::On,
        false => MergeBlockState::OutOfSync,
    }
}
