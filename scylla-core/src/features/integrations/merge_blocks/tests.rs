//! Unit tests of the pure rules of the merge blocks.

use scylla_domain::integrations::merge_blocks::MergeBlockState;
use scylla_domain::integrations::VcsProvider;
use serde_json::json;
use time::OffsetDateTime;

use super::rules::{self, Observed, Refusal, Ruleset};
use crate::infra::vcs::ProviderError;

/// The report of a pipeline of the editor has the key of its name, and a file pipeline has its
/// file key.
#[test]
fn context_names_the_report_of_a_pipeline() {
    // when
    let editor = rules::context("Build and test", None);
    let file = rules::context("Build", Some("deploy"));

    // then
    assert_eq!(editor, "scylla/build-and-test");
    assert_eq!(file, "scylla/deploy");
}

/// A rule to turn on always calls the provider. A rule to turn off calls it only when the
/// provider can have the rule.
#[test]
fn needs_call_skips_a_rule_that_the_provider_does_not_have() {
    // given
    let skipped = [
        None,
        Some(MergeBlockState::Off),
        Some(MergeBlockState::NeedsPermission),
        Some(MergeBlockState::NotPossible),
    ];
    let called = [
        MergeBlockState::On,
        MergeBlockState::Applying,
        MergeBlockState::OutOfSync,
        MergeBlockState::Failed,
    ];

    // then
    assert!(skipped
        .iter()
        .all(|state| !rules::needs_call(*state, false)));
    assert!(skipped.iter().all(|state| rules::needs_call(*state, true)));
    assert!(called
        .iter()
        .all(|state| rules::needs_call(Some(*state), false)));
}

/// A refusal of GitHub is a missing permission, or the limit of GitHub Free on a private
/// repository. A refusal of GitLab is a role below Maintainer. Other failures fail with a code.
#[test]
fn refusal_maps_the_errors_of_the_provider() {
    // given
    let github = VcsProvider::GitHub;
    let gitlab = VcsProvider::GitLab;
    let limit = ProviderError::RateLimited(OffsetDateTime::UNIX_EPOCH);
    let down = ProviderError::Transient("status 502".to_owned());

    // when
    let public = rules::refusal(github, false, &ProviderError::Forbidden);
    let private = rules::refusal(github, true, &ProviderError::Forbidden);
    let role = rules::refusal(gitlab, true, &ProviderError::Forbidden);
    let limited = rules::refusal(gitlab, false, &limit);
    let unavailable = rules::refusal(github, false, &down);
    let refused = rules::refusal(github, false, &ProviderError::NotFound);

    // then
    let needs = MergeBlockState::NeedsPermission;
    let failed = MergeBlockState::Failed;
    assert_eq!(
        public,
        Refusal::new(needs, "ADMINISTRATION_PERMISSION_MISSING")
    );
    assert_eq!(
        private,
        Refusal::new(
            MergeBlockState::NotPossible,
            "PRIVATE_REPOSITORY_NOT_ALLOWED"
        )
    );
    assert_eq!(role, Refusal::new(needs, "ROLE_TOO_LOW"));
    assert_eq!(limited, Refusal::new(failed, "PROVIDER_LIMIT"));
    assert_eq!(unavailable, Refusal::new(failed, "PROVIDER_UNAVAILABLE"));
    assert_eq!(refused, Refusal::new(failed, "PROVIDER_REFUSED"));
}

/// The ruleset of Scylla targets the default branch, and requires each context from the GitHub
/// App, with no strict policy. Two pipelines with one report name give one check.
#[test]
fn ruleset_body_requires_each_context_from_the_app() {
    // given
    let contexts =
        rules::unique(["scylla/build", "scylla/deploy", "scylla/build"].map(String::from));

    // when
    let body = rules::ruleset_body(&contexts, 42);

    // then
    assert_eq!(
        body,
        json!({
            "name": "Scylla required checks",
            "target": "branch",
            "enforcement": "active",
            "conditions": { "ref_name": { "include": ["~DEFAULT_BRANCH"], "exclude": [] } },
            "rules": [{
                "type": "required_status_checks",
                "parameters": {
                    "strict_required_status_checks_policy": false,
                    "required_status_checks": [
                        { "context": "scylla/build", "integration_id": 42 },
                        { "context": "scylla/deploy", "integration_id": 42 },
                    ],
                },
            }],
        })
    );
}

/// A ruleset requires its contexts only when it is active on the default branch.
#[test]
fn required_reads_an_active_ruleset_only() {
    // given
    let active = Ruleset {
        enforcement: "active".to_owned(),
        include: vec!["~DEFAULT_BRANCH".to_owned()],
        contexts: vec!["scylla/build".to_owned()],
    };
    let evaluate = Ruleset {
        enforcement: "evaluate".to_owned(),
        ..active.clone()
    };
    let other_branch = Ruleset {
        include: vec!["refs/heads/dev".to_owned()],
        ..active.clone()
    };

    // then
    assert_eq!(
        rules::required(Some(&active))
            .into_iter()
            .collect::<Vec<_>>(),
        ["scylla/build"]
    );
    assert!(rules::required(Some(&evaluate)).is_empty());
    assert!(rules::required(Some(&other_branch)).is_empty());
    assert!(rules::required(None).is_empty());
}

/// A rule that is on goes out of sync when the provider does not have it, or when its report
/// has a new name. An unknown provider keeps the state, and a rule that is not on keeps it.
#[test]
fn observed_finds_a_rule_out_of_sync() {
    // given
    let on = MergeBlockState::On;
    let build = Some("scylla/build");
    let has = Observed::GitHub(["scylla/build".to_owned()].into());
    let lacks = Observed::GitHub(["scylla/deploy".to_owned()].into());

    // when
    let kept = rules::observed(on, true, build, build, &has);
    let removed = rules::observed(on, true, build, build, &lacks);
    let renamed = rules::observed(on, true, Some("scylla/old"), build, &has);
    let unknown = rules::observed(on, true, build, build, &Observed::Unknown);
    let gitlab_off = rules::observed(on, true, None, None, &Observed::GitLab(false));
    let gitlab_on = rules::observed(on, true, None, None, &Observed::GitLab(true));
    let needs = MergeBlockState::NeedsPermission;
    let waiting = rules::observed(needs, true, build, build, &lacks);

    // then
    let out = MergeBlockState::OutOfSync;
    assert_eq!([kept, unknown, gitlab_on], [on, on, on]);
    assert_eq!([removed, renamed, gitlab_off], [out, out, out]);
    assert_eq!(waiting, needs);
}
