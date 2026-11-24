//! The tests of the ruleset of Scylla on GitHub: its creation, its contexts, its delete, the
//! refusals of GitHub, and a ruleset that a person changed.

use scylla_proto::integrations::v1::{MergeBlock, MergeBlockState};
use scylla_testkit::{exec_node, text, TestApp};
use serde_json::{json, Value};
use sqlx::PgPool;

use super::{app_with_fake, linked, Linked};

/// Returns the required checks of the body of a ruleset.
fn checks(body: &Value) -> &Value {
    &body["rules"][0]["parameters"]["required_status_checks"]
}

/// Turns the rule of a pipeline on or off as the owner.
async fn set(app: &TestApp, linked: &Linked, pipeline: &str, enabled: bool) -> MergeBlock {
    app.set_merge_block(linked.owner, &linked.project, Some(pipeline), enabled)
        .await
        .unwrap()
}

/// Returns the state of the first rule of the list of the owner.
async fn first_state(app: &TestApp, linked: &Linked) -> i32 {
    let listed = app.list_merge_blocks(linked.owner, &linked.project).await;
    listed.unwrap()[0].state
}

/// Story 8.1, step 7: an editor turns on the rule of the pipeline `build`. Scylla makes the
/// ruleset "Scylla required checks" on the default branch with the check `scylla/build` of the
/// App, and the rule is on.
#[sqlx::test(migrations = false)]
async fn set_merge_block_makes_the_ruleset_on_github(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    fake.answer_github_rules_token(42);
    fake.answer_github_rulesets("octo/app", 99);
    fake.answer_github_ruleset("octo/app", 99, "active", &["scylla/build"]);
    let pipeline = Some(linked.pipeline.as_str());

    // when
    let block = app
        .set_merge_block(linked.owner, &linked.project, pipeline, true)
        .await
        .unwrap();
    let listed = app.list_merge_blocks(linked.owner, &linked.project).await;

    // then
    assert_eq!(block.state, MergeBlockState::On as i32);
    assert!(block.enabled && block.state_reason.is_empty());
    assert_eq!(block.context, "scylla/build");
    assert_eq!(block.branch, "main");
    assert_eq!(block.action_url, "https://github.com/octo/app/rules/99");
    assert!(block.updated_at.is_some());
    assert_eq!(listed.unwrap(), vec![block]);
    let tokens = fake.requests_to("POST", "/app/installations/42/access_tokens");
    let wanted = json!({ "repository_ids": [7], "permissions": { "administration": "write" } });
    assert_eq!(tokens[0].body, wanted);
    let made = fake.requests_to("POST", "/repos/octo/app/rulesets");
    assert_eq!(made.len(), 1);
    let body = &made[0].body;
    assert_eq!(body["name"], "Scylla required checks");
    assert_eq!(
        (&body["target"], &body["enforcement"]),
        (&json!("branch"), &json!("active"))
    );
    let include = &body["conditions"]["ref_name"]["include"];
    assert_eq!(include, &json!(["~DEFAULT_BRANCH"]));
    let strict = &body["rules"][0]["parameters"]["strict_required_status_checks_policy"];
    assert_eq!(strict, &json!(false));
    let wanted = json!([{ "context": "scylla/build", "integration_id": 1 }]);
    assert_eq!(checks(body), &wanted);
    assert_eq!(app.ruleset_of(&linked.project).await, Some(99));
}

/// Each toggle changes the list of the contexts of the one ruleset, and the last rule that goes
/// off removes the ruleset.
#[sqlx::test(migrations = false)]
async fn set_merge_block_changes_the_contexts_of_the_ruleset(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let nodes = vec![exec_node("ship", &[])];
    let deploy = app
        .create_pipeline(linked.owner, &linked.project, nodes)
        .await
        .unwrap();
    let deploy = text(&deploy.id).to_owned();
    app.update_pipeline(linked.owner, &deploy, 1, Some("Deploy"), None)
        .await
        .unwrap();
    fake.answer_github_rules_token(42);
    fake.answer_github_rulesets("octo/app", 99);

    // when
    set(&app, &linked, &linked.pipeline, true).await;
    let both = set(&app, &linked, &deploy, true).await;
    set(&app, &linked, &linked.pipeline, false).await;
    let last = set(&app, &linked, &deploy, false).await;
    let listed = app
        .list_merge_blocks(linked.owner, &linked.project)
        .await
        .unwrap();

    // then
    assert_eq!(both.state, MergeBlockState::On as i32);
    assert_eq!(both.context, "scylla/deploy");
    let changes = fake.requests_to("PUT", "/repos/octo/app/rulesets/99");
    let contexts: Vec<Value> = changes
        .iter()
        .map(|change| checks(&change.body).clone())
        .collect();
    let build = json!({ "context": "scylla/build", "integration_id": 1 });
    let ship = json!({ "context": "scylla/deploy", "integration_id": 1 });
    assert_eq!(contexts, vec![json!([build, ship]), json!([ship])]);
    assert_eq!(
        fake.requests_to("DELETE", "/repos/octo/app/rulesets/99")
            .len(),
        1
    );
    assert_eq!(last.state, MergeBlockState::Off as i32);
    assert_eq!(
        last.action_url,
        "https://github.com/octo/app/settings/rules"
    );
    assert_eq!(app.ruleset_of(&linked.project).await, None);
    let states: Vec<_> = listed
        .iter()
        .map(|block| (block.enabled, block.state))
        .collect();
    let off = (false, MergeBlockState::Off as i32);
    assert_eq!(states, vec![off, off]);
}

/// L3-6: an installation with no `administration: write` gets no token for the rule. The rule
/// needs the permission, with the page of the installation, and nothing changes on GitHub. The
/// rule then goes off with no call.
#[sqlx::test(migrations = false)]
async fn set_merge_block_needs_the_administration_permission(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let refused =
        json!({ "message": "The permissions requested are not granted to this installation." });
    fake.respond(
        "POST",
        "/app/installations/42/access_tokens",
        422,
        refused,
        &[],
    );
    let pipeline = Some(linked.pipeline.as_str());

    // when
    let on = app
        .set_merge_block(linked.owner, &linked.project, pipeline, true)
        .await
        .unwrap();
    let calls = fake.requests().len();
    let off = app
        .set_merge_block(linked.owner, &linked.project, pipeline, false)
        .await
        .unwrap();

    // then
    assert!(on.enabled);
    assert_eq!(on.state, MergeBlockState::NeedsPermission as i32);
    assert_eq!(on.state_reason, "ADMINISTRATION_PERMISSION_MISSING");
    let page = format!(
        "{}/organizations/octo/settings/installations/42",
        fake.url()
    );
    assert_eq!(on.action_url, page);
    assert!(fake
        .requests_to("POST", "/repos/octo/app/rulesets")
        .is_empty());
    assert_eq!(
        (off.enabled, off.state),
        (false, MergeBlockState::Off as i32)
    );
    assert!(off.state_reason.is_empty());
    assert_eq!(fake.requests().len(), calls);
}

/// GitHub Free allows no rule on a private repository: the rule is not possible. A failure of
/// GitHub fails the rule with a code. Nothing changes on GitHub.
#[sqlx::test(migrations = false)]
async fn set_merge_block_keeps_the_refusals_of_github(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    app.set_link_access(&linked.project, true, None).await;
    fake.answer_github_rules_token(42);
    fake.respond("GET", "/repos/octo/app/rulesets", 200, json!([]), &[]);
    let upgrade = json!({ "message": "Upgrade to GitHub Pro or make this repository public to enable this feature." });
    fake.respond("POST", "/repos/octo/app/rulesets", 403, upgrade, &[]);
    fake.respond("POST", "/repos/octo/app/rulesets", 502, Value::Null, &[]);

    // when
    let private = set(&app, &linked, &linked.pipeline, true).await;
    let down = set(&app, &linked, &linked.pipeline, true).await;

    // then
    assert_eq!(private.state, MergeBlockState::NotPossible as i32);
    assert_eq!(private.state_reason, "PRIVATE_REPOSITORY_NOT_ALLOWED");
    assert_eq!(down.state, MergeBlockState::Failed as i32);
    assert_eq!(down.state_reason, "PROVIDER_UNAVAILABLE");
    assert!(down.enabled);
    assert_eq!(app.ruleset_of(&linked.project).await, None);
}

/// A list reads the ruleset: a ruleset that a person turned off, or a pipeline with a new name,
/// gives a rule out of sync. A new change applies the rule of Scylla again.
#[sqlx::test(migrations = false)]
async fn list_merge_blocks_finds_a_ruleset_out_of_sync(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    fake.answer_github_rules_token(42);
    fake.answer_github_rulesets("octo/app", 99);
    fake.answer_github_ruleset("octo/app", 99, "disabled", &["scylla/build"]);
    fake.answer_github_ruleset("octo/app", 99, "active", &["scylla/build"]);
    set(&app, &linked, &linked.pipeline, true).await;

    // when
    let disabled = first_state(&app, &linked).await;
    let active = first_state(&app, &linked).await;
    app.update_pipeline(linked.owner, &linked.pipeline, 1, Some("Release"), None)
        .await
        .unwrap();
    let renamed = first_state(&app, &linked).await;
    let again = set(&app, &linked, &linked.pipeline, true).await;

    // then
    let out = MergeBlockState::OutOfSync as i32;
    assert_eq!(
        [disabled, active, renamed],
        [out, MergeBlockState::On as i32, out]
    );
    assert_eq!(
        (again.state, again.context.as_str()),
        (MergeBlockState::On as i32, "scylla/release")
    );
    let changes = fake.requests_to("PUT", "/repos/octo/app/rulesets/99");
    let wanted = json!([{ "context": "scylla/release", "integration_id": 1 }]);
    assert_eq!(checks(&changes[0].body), &wanted);
}
