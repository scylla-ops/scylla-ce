//! Unit tests of the Git events: their refs, the input of the rules and the source of a job.

use super::{EventCommit, EventRepository, GitChange, GitEvent};
use crate::integrations::sources::{PullRequestRef, SourceEvent};
use crate::integrations::VcsProvider;
use crate::projects::git_rules::{PullRequestAction, TriggerEvent};

/// Returns an event of `octo/app` with a change, on a commit with a message or none.
fn event(provider: VcsProvider, change: GitChange, message: Option<&str>) -> GitEvent {
    GitEvent {
        provider,
        key: "delivery-1".to_owned(),
        repository: EventRepository {
            external_id: "7".to_owned(),
            full_name: "octo/app".to_owned(),
            web_url: "https://github.com/octo/app".to_owned(),
        },
        change,
        commit: EventCommit {
            sha: "a".repeat(40),
            message: message.map(str::to_owned),
            author_login: Some("octocat".to_owned()),
            avatar: None,
        },
        polled: false,
    }
}

/// Returns a pull request #42 from `feature` into `main`.
fn pull() -> PullRequestRef {
    PullRequestRef {
        number: 42,
        title: "Add login".to_owned(),
        url: "https://github.com/octo/app/pull/42".to_owned(),
        source_branch: "feature".to_owned(),
        target_branch: "main".to_owned(),
    }
}

/// Returns a change of the pull request #42, from a fork or from a branch of the repository.
fn pull_change(fork: Option<&str>) -> GitChange {
    GitChange::PullRequest {
        pull: pull(),
        action: PullRequestAction::NewCommits,
        draft: false,
        fork_repository: fork.map(str::to_owned),
    }
}

/// The head of a push, a tag and a pull request is a full Git ref, and the head of a pull
/// request has the form of its provider.
#[test]
fn the_head_ref_has_the_form_of_the_provider() {
    // given
    let push = GitChange::Push {
        branch: "release/1".to_owned(),
    };
    let tag = GitChange::Tag {
        name: "v1.0".to_owned(),
    };

    // when
    let refs = [
        event(VcsProvider::GitHub, push, None).head_ref(),
        event(VcsProvider::GitHub, tag, None).head_ref(),
        event(VcsProvider::GitHub, pull_change(None), None).head_ref(),
        event(VcsProvider::GitLab, GitChange::Closed { number: 17 }, None).head_ref(),
    ];

    // then
    assert_eq!(
        refs,
        [
            "refs/heads/release/1",
            "refs/tags/v1.0",
            "refs/pull/42/head",
            "refs/merge-requests/17/head",
        ]
    );
    assert_eq!(
        GitEvent::poll_key("octo/app", "refs/heads/main", "abc"),
        "poll:octo/app:refs/heads/main:abc"
    );
}

/// The rules of the triggers get the branch and the message of a push, the target of a pull
/// request, and nothing for a closed pull request.
#[test]
fn the_rules_get_the_branch_the_target_and_the_message() {
    // given
    let push = GitChange::Push {
        branch: "main".to_owned(),
    };
    let push = event(VcsProvider::GitHub, push, Some("Fix [skip ci]"));
    let pull = event(VcsProvider::GitHub, pull_change(None), None);
    let closed = event(VcsProvider::GitHub, GitChange::Closed { number: 42 }, None);

    // when
    let (push, pull, closed) = (push.trigger(), pull.trigger(), closed.trigger());

    // then
    assert_eq!(
        push,
        Some(TriggerEvent::Push {
            branch: "main",
            message: "Fix [skip ci]"
        })
    );
    assert_eq!(
        pull,
        Some(TriggerEvent::PullRequest {
            target: "main",
            action: PullRequestAction::NewCommits,
            draft: false
        })
    );
    assert_eq!(closed, None);
}

/// The source of a push is automatic and has its branch and the title of its message. A pull
/// request from a fork has no branch and names the fork, and its title is the title of the pull
/// request when the provider gives no message.
#[test]
fn the_source_of_a_job_names_the_branch_or_the_fork() {
    // given
    let push = GitChange::Push {
        branch: "main".to_owned(),
    };
    let push = event(
        VcsProvider::GitHub,
        push,
        Some("Fix the build\n\nMore text"),
    );
    let fork = event(VcsProvider::GitHub, pull_change(Some("eve/app")), None);
    let closed = event(VcsProvider::GitHub, GitChange::Closed { number: 42 }, None);

    // when
    let push = push.source().unwrap();
    let fork = fork.source().unwrap();

    // then
    assert_eq!(
        (push.event, push.automatic, push.message_title.as_str()),
        (SourceEvent::Push, true, "Fix the build")
    );
    assert_eq!(push.branch.as_deref(), Some("main"));
    assert_eq!(push.git_ref.as_deref(), Some("refs/heads/main"));
    assert_eq!(fork.branch, None);
    assert_eq!(fork.fork_repository.as_deref(), Some("eve/app"));
    assert_eq!(fork.pull_requests, vec![pull()]);
    assert_eq!(fork.message_title, "Add login");
    assert!(fork.from_fork());
    assert_eq!(closed.source(), None);
}
