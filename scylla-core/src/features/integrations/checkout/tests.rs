//! Unit tests of the checkout: the ref of a commit of a fork, and the header of a token.

use scylla_domain::integrations::sources::{JobSource, PullRequestRef, SourceEvent};
use scylla_domain::integrations::VcsProvider;

use super::dispatch::{basic, fetch_ref};
use crate::infra::vcs::Hidden;

/// Makes the source of a pull request 57 on a provider, from a fork if `fork` is set.
fn source(provider: VcsProvider, fork: Option<&str>) -> JobSource {
    JobSource {
        provider,
        repository: "acme/api".to_owned(),
        url: "https://example.com/acme/api".to_owned(),
        sha: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        git_ref: None,
        branch: Some("feature".to_owned()),
        tag: None,
        pull_requests: vec![PullRequestRef {
            number: 57,
            title: "Fix".to_owned(),
            url: "https://example.com/acme/api/pull/57".to_owned(),
            source_branch: "feature".to_owned(),
            target_branch: "main".to_owned(),
        }],
        fork_repository: fork.map(str::to_owned),
        author_login: None,
        avatar: None,
        message_title: "Fix".to_owned(),
        event: SourceEvent::PullRequest,
        automatic: true,
        changes_pipeline_files: false,
    }
}

/// A commit of a fork comes from the ref of its pull request in the base repository. Another
/// commit comes by its SHA.
#[test]
fn a_fork_fetches_the_ref_of_its_pull_request() {
    // given
    let github = source(VcsProvider::GitHub, Some("evil/api"));
    let gitlab = source(VcsProvider::GitLab, Some("evil/api"));
    let same = source(VcsProvider::GitHub, None);

    // when
    let refs = [fetch_ref(&github), fetch_ref(&gitlab), fetch_ref(&same)];

    // then
    assert_eq!(
        refs,
        [
            "refs/pull/57/head".to_owned(),
            "refs/merge-requests/57/head".to_owned(),
            String::new(),
        ]
    );
}

/// The header of a token is a basic authentication of the user and the token, and `Debug` does
/// not show it.
#[test]
fn a_header_is_a_basic_authentication() {
    // when
    let header = basic("x-access-token", &Hidden::new("ghs_test"));

    // then
    assert_eq!(
        header.expose(),
        "Authorization: Basic eC1hY2Nlc3MtdG9rZW46Z2hzX3Rlc3Q="
    );
    assert_eq!(format!("{header:?}"), "***");
}
