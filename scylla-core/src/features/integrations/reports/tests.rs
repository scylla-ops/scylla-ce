//! Unit tests of the reports: the map of the states, the descriptions, the waits and what comes
//! after a failed call.

use scylla_domain::integrations::reports::ReportedStatus;
use scylla_domain::integrations::VcsProvider;
use time::{Duration, OffsetDateTime};

use super::failures::{wait, Failure, Next};
use super::states::{self, Run};
use crate::infra::vcs::ProviderError;

/// Each state of a run has its state on GitHub and on GitLab, as in the map of the reports.
#[test]
fn each_state_of_a_run_has_a_state_on_each_provider() {
    // given
    let all = ReportedStatus::ALL;

    // when
    let github: Vec<&str> = all.iter().map(|status| states::github(*status)).collect();
    let gitlab: Vec<&str> = all.iter().map(|status| states::gitlab(*status)).collect();

    // then
    let github_map = [
        "pending", "pending", "pending", "success", "failure", "error", "error", "failure",
    ];
    let gitlab_map = [
        "pending", "pending", "running", "success", "failed", "canceled", "failed", "failed",
    ];
    assert_eq!(github, github_map);
    assert_eq!(gitlab, gitlab_map);
}

/// A description tells the state with its duration, its failed node or its file, and stays in
/// the limit of the provider.
#[test]
fn a_description_tells_the_state_of_the_run() {
    // given
    let start = OffsetDateTime::UNIX_EPOCH;
    let ended = Run {
        started_at: Some(start),
        finished_at: Some(start + Duration::seconds(134)),
        failed_node: Some("test"),
        file_path: Some(".scylla/pipelines/build.toml"),
    };
    let long = "x".repeat(300);
    let invalid = Run {
        file_path: Some(&long),
        ..Run::default()
    };

    // when
    let text = |status| states::description(VcsProvider::GitHub, status, &ended);
    let cut_github =
        states::description(VcsProvider::GitHub, ReportedStatus::InvalidFile, &invalid);
    let cut_gitlab =
        states::description(VcsProvider::GitLab, ReportedStatus::InvalidFile, &invalid);

    // then
    assert_eq!(
        text(ReportedStatus::AwaitingApproval),
        "Waiting for approval in Scylla"
    );
    assert_eq!(text(ReportedStatus::Pending), "Queued");
    assert_eq!(text(ReportedStatus::Completed), "Passed in 2m 14s");
    assert_eq!(text(ReportedStatus::Failed), "Failed: node test");
    assert_eq!(text(ReportedStatus::Orphaned), "Agent lost");
    assert_eq!(
        text(ReportedStatus::InvalidFile),
        "Invalid .scylla/pipelines/build.toml"
    );
    assert_eq!(cut_github.chars().count(), 140);
    assert_eq!(cut_gitlab.chars().count(), 255);
}

/// A duration shows in seconds, in minutes and seconds, or in hours and minutes.
#[test]
fn a_duration_is_short() {
    // given
    let lengths = [58, 134, 3780].map(Duration::seconds);

    // when
    let texts = lengths.map(states::length);

    // then
    assert_eq!(texts, ["58s", "2m 14s", "1h 3m"]);
}

/// The wait after a failed call doubles from 10 seconds, up to 15 minutes.
#[test]
fn the_wait_doubles_up_to_a_limit() {
    // given
    let attempts = [0, 1, 2, 6, 7, 30];

    // when
    let waits = attempts.map(|count| wait(count).whole_seconds());

    // then
    assert_eq!(waits, [10, 20, 40, 640, 900, 900]);
}

/// A limit waits for its end, a transient failure waits, a refused call breaks the connection
/// with a reason, and a refused request or a failure of the server stops the report.
#[test]
fn a_failed_call_gives_the_next_step() {
    // given
    let now = OffsetDateTime::UNIX_EPOCH;
    let until = now + Duration::minutes(5);
    let failure = |error| Failure::Provider(error);
    let github = |error| failure(error).next(VcsProvider::GitHub, 0, false, now);
    let gitlab = |error, expired| failure(error).next(VcsProvider::GitLab, 2, expired, now);
    let server = Failure::Server("no App".to_owned());

    // when
    let limited = github(ProviderError::RateLimited(until));
    let transient = gitlab(ProviderError::Transient("status 502".to_owned()), false);
    let uninstalled = github(ProviderError::NotFound);
    let forbidden = github(ProviderError::Forbidden);
    let expired = gitlab(ProviderError::Unauthorized, true);
    let revoked = gitlab(ProviderError::Unauthorized, false);
    let scope = gitlab(ProviderError::Forbidden, false);
    let invalid = github(ProviderError::Invalid("bad".to_owned()));
    let stopped = server.next(VcsProvider::GitHub, 0, false, now);

    // then
    assert_eq!(limited, Next::Retry(until));
    assert_eq!(transient, Next::Retry(now + Duration::seconds(40)));
    assert_eq!(uninstalled, Next::Broken("APP_UNINSTALLED"));
    assert_eq!(forbidden, Next::Broken("PERMISSIONS_NOT_ACCEPTED"));
    assert_eq!(expired, Next::Broken("TOKEN_EXPIRED"));
    assert_eq!(revoked, Next::Broken("TOKEN_REVOKED"));
    assert_eq!(scope, Next::Broken("TOKEN_SCOPE_MISSING"));
    assert_eq!(invalid, Next::Stop);
    assert_eq!(stopped, Next::Stop);
}
