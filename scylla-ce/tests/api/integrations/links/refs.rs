//! The tests of the branches and the tags of a linked repository: the default branch first, the
//! filters of kind and text, and the pages.

use scylla_proto::integrations::v1::{GitRefInfo, GitRefKind};
use sqlx::PgPool;

use super::{app_with_fake, assert_reason, insert_link, project};

/// Returns the names of some refs.
fn names(refs: &[GitRefInfo]) -> Vec<&str> {
    refs.iter().map(|item| item.name.as_str()).collect()
}

/// The branches come first with the default branch at the top, then the tags. A filter of kind
/// and a text in any case keep some refs, and a page of one item gives the next token.
#[sqlx::test(migrations = false)]
async fn list_refs_gives_the_default_branch_first(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let project = project(&app, "github").await;
    insert_link(&app, &project, "octo/app").await;
    let branches = [("develop", "d1"), ("main", "a1"), ("release/1", "r1")];
    fake.answer_github_refs(42, "octo/app", &branches, &[("v1.0", "t1")]);
    let (owner, id) = (project.owner, project.id.as_str());

    // when
    let all = app.list_refs(owner, id, GitRefKind::Unspecified, "", 0, "");
    let all = all.await.unwrap();
    let tags = app.list_refs(owner, id, GitRefKind::Tag, "", 0, "").await;
    let found = app.list_refs(owner, id, GitRefKind::Unspecified, "E", 0, "");
    let found = found.await.unwrap();
    let first = app
        .list_refs(owner, id, GitRefKind::Branch, "", 1, "")
        .await;
    let first = first.unwrap();
    let token = first.page.clone().unwrap_or_default().next_token;
    let second = app.list_refs(owner, id, GitRefKind::Branch, "", 1, &token);

    // then
    assert_eq!(names(&all.refs), ["main", "develop", "release/1", "v1.0"]);
    let main = &all.refs[0];
    assert!(main.default_branch && main.kind == GitRefKind::Branch as i32);
    assert_eq!(main.sha, "a1");
    assert_eq!(main.web_url, "https://github.com/octo/app/tree/main");
    assert_eq!(all.refs[3].kind, GitRefKind::Tag as i32);
    assert!(all.page.unwrap_or_default().next_token.is_empty());
    assert_eq!(names(&tags.unwrap().refs), ["v1.0"]);
    assert_eq!(names(&found.refs), ["develop", "release/1"]);
    assert_eq!(names(&first.refs), ["main"]);
    assert_eq!(names(&second.await.unwrap().refs), ["develop"]);
    let tokens = fake.requests_to("POST", "/app/installations/42/access_tokens");
    let narrow = serde_json::json!({
        "repository_ids": [7],
        "permissions": { "contents": "read" },
    });
    assert_eq!(tokens[0].body, narrow);
}

/// On GitLab, the search goes to GitLab, and a tag gets the page of the tags of the project.
#[sqlx::test(migrations = false)]
async fn list_refs_searches_on_gitlab(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let project = project(&app, "gitlab").await;
    insert_link(&app, &project, "group/app").await;
    fake.answer_gitlab_refs(7, &[("release/2", "r2")], &[("release-2.0", "t2")]);

    // when
    let found = app
        .list_refs(
            project.owner,
            &project.id,
            GitRefKind::Unspecified,
            "release",
            0,
            "",
        )
        .await
        .unwrap();

    // then
    assert_eq!(names(&found.refs), ["release/2", "release-2.0"]);
    assert!(!found.refs[0].default_branch);
    let tag = &found.refs[1].web_url;
    assert_eq!(tag, "https://gitlab.com/group/app/-/tags/release-2.0");
    let branches = fake.requests_to("GET", "/api/v4/projects/7/repository/branches");
    assert!(
        branches[0].query.contains("search=release"),
        "{:?}",
        branches[0]
    );
}

/// A project with no link gives `NO_REPOSITORY_LINK`.
#[sqlx::test(migrations = false)]
async fn list_refs_needs_a_link(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let project = project(&app, "github").await;

    // when
    let refused = app
        .list_refs(project.owner, &project.id, GitRefKind::Branch, "", 0, "")
        .await;

    // then
    assert_reason(refused, "NO_REPOSITORY_LINK");
}
