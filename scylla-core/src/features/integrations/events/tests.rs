//! Unit tests of the payloads of the webhooks: the Git event, the re-run or the change of an
//! installation of each one. The runs of the events come later, so the API cannot show them.

use scylla_domain::integrations::events::GitChange;
use scylla_domain::projects::git_rules::PullRequestAction;
use serde_json::{json, Value};

use super::dispatch::Incoming;
use super::installations::InstallationChange;
use super::{github, gitlab};

/// Returns the GitHub payload of an action on the pull request #5 of `octo/app`, from a branch
/// of the repository `head` (7 for the repository itself).
fn github_pull(action: &str, head: u64) -> Value {
    json!({
        "action": action,
        "number": 5,
        "installation": { "id": 42 },
        "repository": { "id": 7, "full_name": "octo/app", "html_url": "https://github.com/octo/app" },
        "pull_request": {
            "number": 5, "title": "Add login", "html_url": "https://github.com/octo/app/pull/5",
            "draft": true,
            "head": { "ref": "login", "sha": "abc", "repo": { "id": head, "full_name": "eve/app" } },
            "base": { "ref": "main", "sha": "def", "repo": { "id": 7, "full_name": "octo/app" } },
            "user": { "login": "eve", "avatar_url": null },
        },
    })
}

/// Reads a GitHub payload of an event.
fn read_github(event: &str, body: &Value) -> Incoming {
    let body = body.to_string();
    github::read(event, "d-1", body.as_bytes())
        .unwrap()
        .incoming
}

/// Reads a GitLab payload.
fn read_gitlab(body: &Value) -> Incoming {
    let body = body.to_string();
    gitlab::read("k-1", body.as_bytes()).unwrap().incoming
}

/// Returns the change of a Git event, or `None` for another delivery.
fn change(incoming: Incoming) -> Option<GitChange> {
    match incoming {
        Incoming::Git(event) => Some(event.change),
        _ => None,
    }
}

/// A pull request from a fork names the fork, `synchronize` has new commits, `closed` closes it,
/// and an action that starts no run gives nothing.
#[test]
fn a_github_pull_request_names_its_fork() {
    // given
    let fork = github_pull("synchronize", 9);
    let closed = github_pull("closed", 7);
    let labeled = github_pull("labeled", 7);

    // when
    let fork = change(read_github("pull_request", &fork));
    let closed = change(read_github("pull_request", &closed));
    let labeled = change(read_github("pull_request", &labeled));

    // then
    let Some(GitChange::PullRequest {
        action,
        draft,
        fork_repository,
        pull,
    }) = fork
    else {
        panic!("a pull request");
    };
    assert_eq!(action, PullRequestAction::NewCommits);
    assert!(draft);
    assert_eq!(fork_repository.as_deref(), Some("eve/app"));
    assert_eq!(
        (pull.source_branch.as_str(), pull.target_branch.as_str()),
        ("login", "main")
    );
    assert_eq!(closed, Some(GitChange::Closed { number: 5 }));
    assert_eq!(labeled, None);
}

/// A push of a tag is a new tag, and a push that deletes a branch gives nothing.
#[test]
fn a_github_push_of_a_tag_is_a_tag() {
    // given
    let push = |git_ref: &str, after: &str, deleted: bool| {
        json!({
            "ref": git_ref, "after": after, "deleted": deleted,
            "repository": { "id": 7, "full_name": "octo/app" }, "installation": { "id": 42 },
        })
    };

    // when
    let tag = change(read_github("push", &push("refs/tags/v1.0", "abc", false)));
    let gone = change(read_github("push", &push("refs/heads/old", "0", true)));

    // then
    assert_eq!(
        tag,
        Some(GitChange::Tag {
            name: "v1.0".to_owned()
        })
    );
    assert_eq!(gone, None);
}

/// A rerequested check run asks for a re-run of its job, and a change of the repositories of an
/// installation lists the added and the removed ones.
#[test]
fn a_github_check_and_an_installation_are_read() {
    // given
    let check = json!({
        "action": "rerequested", "installation": { "id": 42 },
        "repository": { "id": 7, "full_name": "octo/app" },
        "check_run": { "head_sha": "abc", "external_id": "job-1" },
        "sender": { "login": "eve" },
    });
    let repositories = json!({
        "action": "removed", "installation": { "id": 42 }, "repository_selection": "selected",
        "repositories_added": [], "repositories_removed": [{ "id": 7, "full_name": "octo/app" }],
    });

    // when
    let check = read_github("check_run", &check);
    let repositories = read_github("installation_repositories", &repositories);

    // then
    let Incoming::Rerun(request) = check else {
        panic!("a re-run");
    };
    assert_eq!(
        (request.sha.as_str(), request.check.as_deref()),
        ("abc", Some("job-1"))
    );
    let Incoming::Installation(change) = repositories else {
        panic!("a change of an installation");
    };
    assert_eq!(
        change,
        InstallationChange::Repositories {
            installation: "42".to_owned(),
            all: false,
            added: Vec::new(),
            removed: vec!["7".to_owned()],
        }
    );
}

/// A GitLab update with new commits has new commits, an update out of draft is ready for
/// review, and an update of the title gives nothing.
#[test]
fn a_gitlab_merge_request_update_is_read() {
    // given
    let request = |oldrev: Option<&str>, changes: Value| {
        json!({
            "object_kind": "merge_request",
            "project": { "id": 7, "path_with_namespace": "group/app" },
            "object_attributes": {
                "iid": 17, "title": "Add login", "url": "", "source_branch": "login",
                "target_branch": "main", "source_project_id": 7, "target_project_id": 7,
                "action": "update", "oldrev": oldrev, "last_commit": { "id": "abc", "message": "Fix" },
            },
            "changes": changes,
        })
    };
    let ready = json!({ "draft": { "previous": true, "current": false } });

    // when
    let commits = change(read_gitlab(&request(Some("old"), json!({}))));
    let ready = change(read_gitlab(&request(None, ready)));
    let title = change(read_gitlab(&request(None, json!({ "title": {} }))));

    // then
    let action = |change: Option<GitChange>| match change {
        Some(GitChange::PullRequest { action, .. }) => Some(action),
        _ => None,
    };
    assert_eq!(action(commits), Some(PullRequestAction::NewCommits));
    assert_eq!(action(ready), Some(PullRequestAction::ReadyForReview));
    assert_eq!(title, None);
}
