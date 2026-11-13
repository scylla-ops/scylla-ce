//! The tests of the rule of a GitLab project: "Pipelines must succeed", the role of the service
//! account, and a setting that a person changed.

use scylla_proto::integrations::v1::MergeBlockState;
use serde_json::json;
use sqlx::PgPool;

use super::{app_with_fake, linked, GITLAB_TOKEN};
use crate::assert_invalid_field;

/// Story 8.2, step 4: an editor turns on the rule of the project. Scylla sets "Pipelines must
/// succeed" with the token of the service account. A list reads the setting: a setting that a
/// person turned off gives a rule out of sync. A rule of a pipeline is a bad request.
#[sqlx::test(migrations = false)]
async fn set_merge_block_sets_pipelines_must_succeed_on_gitlab(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "gitlab").await;
    fake.answer_gitlab_merge_rule(7, 200);
    fake.answer_gitlab_merge_setting(7, true);
    fake.answer_gitlab_merge_setting(7, false);
    let (owner, project) = (linked.owner, linked.project.as_str());

    // when
    let empty = app.list_merge_blocks(owner, project).await.unwrap();
    let on = app
        .set_merge_block(owner, project, None, true)
        .await
        .unwrap();
    let kept = app.list_merge_blocks(owner, project).await.unwrap();
    let changed = app.list_merge_blocks(owner, project).await.unwrap();
    let pipeline = Some(linked.pipeline.as_str());
    let refused = app.set_merge_block(owner, project, pipeline, true).await;
    let off = app
        .set_merge_block(owner, project, None, false)
        .await
        .unwrap();

    // then
    assert_eq!(empty.len(), 1);
    assert_eq!(
        (empty[0].enabled, empty[0].state),
        (false, MergeBlockState::Off as i32)
    );
    assert!(empty[0].pipeline_id.is_none() && empty[0].updated_at.is_none());
    assert_eq!(on.state, MergeBlockState::On as i32);
    assert!(on.enabled && on.pipeline_id.is_none() && on.context.is_empty());
    assert_eq!(on.branch, "main");
    let settings = "https://gitlab.com/group/api/-/settings/merge_requests";
    assert_eq!(on.action_url, settings);
    assert_eq!(kept, vec![on]);
    assert_eq!(changed[0].state, MergeBlockState::OutOfSync as i32);
    assert_invalid_field(&refused.unwrap_err(), "pipeline_id");
    assert_eq!(off.state, MergeBlockState::Off as i32);
    let puts = fake.requests_to("PUT", "/api/v4/projects/7");
    let bodies: Vec<_> = puts.iter().map(|put| put.body.clone()).collect();
    let rule = |on: bool| json!({ "only_allow_merge_if_pipeline_succeeds": on });
    assert_eq!(bodies, vec![rule(true), rule(false)]);
    assert_eq!(puts[0].header("private-token"), Some(GITLAB_TOKEN));
}

/// L3-6: GitLab refuses the change of a service account below Maintainer. The rule needs the
/// role, with the page of the members, and nothing changes on GitLab.
#[sqlx::test(migrations = false)]
async fn set_merge_block_needs_the_maintainer_role(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "gitlab").await;
    app.set_link_access(&linked.project, true, Some("developer"))
        .await;
    fake.answer_gitlab_merge_rule(7, 403);

    // when
    let refused = app
        .set_merge_block(linked.owner, &linked.project, None, true)
        .await
        .unwrap();
    let listed = app.list_merge_blocks(linked.owner, &linked.project).await;

    // then
    assert!(refused.enabled);
    assert_eq!(refused.state, MergeBlockState::NeedsPermission as i32);
    assert_eq!(refused.state_reason, "ROLE_TOO_LOW");
    let members = "https://gitlab.com/group/api/-/project_members";
    assert_eq!(refused.action_url, members);
    assert_eq!(listed.unwrap(), vec![refused]);
    assert!(fake.requests_to("GET", "/api/v4/projects/7").is_empty());
}
