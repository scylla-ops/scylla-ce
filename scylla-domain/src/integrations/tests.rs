//! Unit tests of the types of the Git integration.

use super::connections::{AccountKind, AuthKind, ConnectionState, DeliveryMode};
use super::links::{GitLabRole, PipelineSource};
use super::merge_blocks::MergeBlockState;
use super::reports::{
    pipeline_key, report_context, ReportMode, ReportState, ReportedStatus, MAX_KEY,
};
use super::sources::{title_of, ApprovalState, GitRef, JobSource, PullRequestRef, SourceEvent};
use super::VcsProvider;
use crate::projects::jobs::JobStatus;
use crate::projects::pipelines::EnvValue;

/// Returns the names of a list of values, then reads each name back.
fn round_trip<T: Copy + PartialEq + std::fmt::Debug>(
    all: &[T],
    as_str: fn(T) -> &'static str,
    parse: fn(&str) -> Option<T>,
) -> Vec<&'static str> {
    all.iter()
        .map(|&value| {
            let name = as_str(value);
            assert_eq!(parse(name), Some(value), "{name} reads back");
            name
        })
        .collect()
}

/// Each value reads back from its name, and the names are the values of the CHECK constraints
/// of the tables.
#[test]
fn each_value_reads_back_from_its_name() {
    // given
    let unknown = "GitHub";

    // when
    let providers = round_trip(VcsProvider::ALL, VcsProvider::as_str, VcsProvider::parse);
    let states = round_trip(
        ConnectionState::ALL,
        ConnectionState::as_str,
        ConnectionState::parse,
    );
    let blocks = round_trip(
        MergeBlockState::ALL,
        MergeBlockState::as_str,
        MergeBlockState::parse,
    );
    let wanted = round_trip(
        ReportedStatus::ALL,
        ReportedStatus::as_str,
        ReportedStatus::parse,
    );
    let events = round_trip(SourceEvent::ALL, SourceEvent::as_str, SourceEvent::parse);
    round_trip(AuthKind::ALL, AuthKind::as_str, AuthKind::parse);
    round_trip(AccountKind::ALL, AccountKind::as_str, AccountKind::parse);
    round_trip(DeliveryMode::ALL, DeliveryMode::as_str, DeliveryMode::parse);
    round_trip(
        PipelineSource::ALL,
        PipelineSource::as_str,
        PipelineSource::parse,
    );
    round_trip(ReportMode::ALL, ReportMode::as_str, ReportMode::parse);
    round_trip(GitLabRole::ALL, GitLabRole::as_str, GitLabRole::parse);
    round_trip(ReportState::ALL, ReportState::as_str, ReportState::parse);
    round_trip(
        ApprovalState::ALL,
        ApprovalState::as_str,
        ApprovalState::parse,
    );

    // then
    assert_eq!(providers, ["github", "gitlab"]);
    assert_eq!(
        states,
        ["active", "suspended", "needs_attention", "disconnected"]
    );
    assert_eq!(blocks.len(), 7);
    assert_eq!(wanted.len(), 8);
    assert_eq!(
        events,
        [
            "push",
            "pull_request",
            "tag",
            "manual",
            "provider_rerun",
            "trigger"
        ]
    );
    assert_eq!(VcsProvider::parse(unknown), None);
}

/// Only an approval, a pending run and a running run can change again.
#[test]
fn a_reported_status_is_final_after_the_run() {
    // given
    let open = [
        ReportedStatus::AwaitingApproval,
        ReportedStatus::Pending,
        ReportedStatus::Running,
    ];

    // when
    let finals: Vec<_> = ReportedStatus::ALL
        .iter()
        .filter(|status| status.is_final())
        .collect();

    // then
    assert!(open.iter().all(|status| !status.is_final()));
    assert_eq!(finals.len(), ReportedStatus::ALL.len() - open.len());
}

/// A report tells the state of its job, and a failed job with errors of its file is an invalid
/// file.
#[test]
fn a_reported_status_follows_the_state_of_the_job() {
    // given
    let failed = JobStatus::Failed;

    // when
    let invalid = ReportedStatus::of_job(failed, true);
    let plain = ReportedStatus::of_job(failed, false);
    let waiting = ReportedStatus::of_job(JobStatus::AwaitingApproval, true);

    // then
    assert_eq!(invalid, ReportedStatus::InvalidFile);
    assert_eq!(plain, ReportedStatus::Failed);
    assert_eq!(waiting, ReportedStatus::AwaitingApproval);
}

/// A report takes a new state, and never a state that is not final after a final one.
#[test]
fn a_report_never_goes_back_from_a_final_state() {
    // given
    let running = ReportedStatus::Running;
    let completed = ReportedStatus::Completed;

    // when
    let forward = running.takes(completed);
    let back = completed.takes(ReportedStatus::Pending);
    let same = running.takes(running);
    let other_final = completed.takes(ReportedStatus::Failed);

    // then
    assert!(forward);
    assert!(!back);
    assert!(!same);
    assert!(other_final);
}

/// The provider sets the mode of a report: a check run on GitHub, a commit status on GitLab.
#[test]
fn a_report_has_the_mode_of_its_provider() {
    // given
    let providers = [VcsProvider::GitHub, VcsProvider::GitLab];

    // when
    let modes = providers.map(ReportMode::of);

    // then
    assert_eq!(modes, [ReportMode::CheckRun, ReportMode::CommitStatus]);
}

/// The name of a report is `scylla/` and the key, and the key of an editor pipeline is the slug
/// of its name.
#[test]
fn a_report_is_named_after_the_key_of_its_pipeline() {
    // given
    let names = ["Build", "Deploy to Prod!", "Tests unitaires é", "  ", "--"];

    // when
    let keys = names.map(pipeline_key);
    let long = pipeline_key(&"a-".repeat(MAX_KEY));

    // then
    assert_eq!(
        keys,
        [
            "build",
            "deploy-to-prod",
            "tests-unitaires-e",
            "pipeline",
            "pipeline"
        ]
    );
    assert_eq!(report_context(&keys[0]), "scylla/build");
    assert!(long.len() <= MAX_KEY && !long.ends_with('-'), "{long}");
}

/// A source gives each variable of the integration, with an empty value when the job does not
/// have it.
#[test]
fn a_source_gives_the_git_variables() {
    // given
    let source = JobSource {
        provider: VcsProvider::GitHub,
        repository: "acme/api".to_owned(),
        url: "https://github.com/acme/api".to_owned(),
        sha: "a".repeat(40),
        git_ref: Some("refs/pull/57/head".to_owned()),
        branch: Some("feature".to_owned()),
        tag: None,
        pull_requests: vec![PullRequestRef {
            number: 57,
            title: "Add login".to_owned(),
            url: "https://github.com/acme/api/pull/57".to_owned(),
            source_branch: "feature".to_owned(),
            target_branch: "main".to_owned(),
        }],
        fork_repository: Some("octocat/api".to_owned()),
        author_login: Some("octocat".to_owned()),
        avatar: None,
        message_title: "Add login".to_owned(),
        event: SourceEvent::PullRequest,
        automatic: true,
        changes_pipeline_files: false,
    };

    // when
    let variables: Vec<(String, String)> = source
        .variables()
        .into_iter()
        .map(|var| match var.value {
            EnvValue::Literal(text) => (var.name, text),
            EnvValue::Secret(_) => panic!("a variable of the source is never a secret"),
        })
        .collect();

    // then
    let expected = [
        ("SCYLLA_GIT_SHA", "a".repeat(40)),
        ("SCYLLA_GIT_REF", "refs/pull/57/head".to_owned()),
        ("SCYLLA_GIT_BRANCH", "feature".to_owned()),
        ("SCYLLA_GIT_TAG", String::new()),
        ("SCYLLA_PR_NUMBER", "57".to_owned()),
        ("SCYLLA_REPO", "acme/api".to_owned()),
        ("SCYLLA_REPO_URL", "https://github.com/acme/api".to_owned()),
        ("SCYLLA_EVENT", "pull_request".to_owned()),
        ("SCYLLA_GIT_FORK", "true".to_owned()),
    ]
    .map(|(name, value)| (name.to_owned(), value));
    assert_eq!(variables, expected);
    assert!(source.from_fork());
}

/// A branch or a tag follows the rules of Git, and a SHA has 7 to 40 hexadecimal characters.
#[test]
fn a_git_ref_follows_the_rules_of_git() {
    // given
    let good = ["main", "release/1.2", "feature/login-page", "v1.2.0", "a"];
    let bad = [
        "", "a b", "-x", "/x", "x/", "a..b", "x.lock", "@", "a@{1}", "a:b", "a~1", "a\\b", ".x",
    ];
    let long = "a".repeat(GitRef::MAX_NAME + 1);

    // when
    let branches: Vec<_> = good
        .iter()
        .map(|name| GitRef::branch((*name).to_owned()))
        .collect();
    let refused: Vec<_> = bad
        .iter()
        .map(|name| GitRef::tag((*name).to_owned()))
        .collect();

    // then
    assert!(branches.iter().all(Result::is_ok));
    assert!(refused.iter().all(Result::is_err), "{refused:?}");
    assert!(GitRef::branch(long).is_err());
    assert_eq!(
        GitRef::sha("ABCDEF1"),
        Ok(GitRef::Sha("abcdef1".to_owned()))
    );
    assert!(GitRef::sha("abcdef").is_err() && GitRef::sha("xyz1234").is_err());
    assert!(GitRef::sha(&"a".repeat(41)).is_err());
    let branch = GitRef::branch("main".to_owned()).unwrap();
    assert_eq!(branch.full_ref().as_deref(), Some("refs/heads/main"));
    let tag = GitRef::tag("v1".to_owned()).unwrap();
    assert_eq!(tag.full_ref().as_deref(), Some("refs/tags/v1"));
    assert_eq!(GitRef::sha("abcdef1").unwrap().full_ref(), None);
    assert_eq!(title_of("Fix the build\n\nMore text"), "Fix the build");
}
