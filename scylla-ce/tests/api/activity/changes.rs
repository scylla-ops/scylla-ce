//! The rows of the changes of a project and of the members: what each row holds, and what
//! records no row.

use scylla_proto::pipelines::v1::trigger;
use scylla_testkit::{pointer_input, text, Role};
use sqlx::PgPool;

use super::place::{kinds, params, Place};
use crate::app;
use crate::projects::triggers::post;

/// The end of a job, each change of a trigger and each change of a secret give one row, newest
/// first, with the actor, the project, the object and the parameters of the kind. A trigger that
/// stays stopped gives no row, and no row holds the value of a secret.
#[sqlx::test(migrations = false)]
async fn list_activity_shows_the_changes_of_a_project(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = Place::new(&app).await;
    let owner = place.owner;

    // when
    let job = place.ended_job(&app, &place.pipeline).await;
    let created = app
        .create_trigger(
            owner,
            &place.pipeline,
            "nightly",
            Some("0 3 * * *"),
            Vec::new(),
        )
        .await
        .unwrap();
    let trigger = text(&created.trigger.unwrap().id).to_owned();
    app.update_trigger(owner, &trigger, Some("0 4 * * *"), None)
        .await
        .unwrap();
    app.set_trigger_enabled(owner, &trigger, false)
        .await
        .unwrap();
    app.set_trigger_enabled(owner, &trigger, false)
        .await
        .unwrap();
    app.delete_trigger(owner, &trigger).await.unwrap();
    let secret = place.secret(&app, "TOKEN").await;
    app.update_secret(owner, &secret, "hidden-second")
        .await
        .unwrap();
    app.delete_secret(owner, &secret).await.unwrap();
    let rows = app.activity_of(owner, &place.workspace).await;

    // then
    let all = [
        "secret.deleted",
        "secret.updated",
        "secret.created",
        "trigger.deleted",
        "trigger.disabled",
        "trigger.updated",
        "trigger.created",
        "job.ended",
    ];
    assert_eq!(kinds(&rows), all);
    let ended = &rows[7];
    assert_eq!(
        (ended.area.as_str(), ended.subject_kind.as_str()),
        ("jobs", "job")
    );
    assert_eq!(text(&ended.subject_id), job);
    assert_eq!(text(&ended.actor_id), owner.to_string());
    assert_eq!(text(&ended.project_id), place.project);
    assert_eq!(ended.actor_name, "Test User");
    assert_eq!(ended.project_name, "alpha");
    let job_params = [
        ("origin", "api"),
        ("pipeline", "build"),
        ("status", "cancelled"),
    ];
    assert_eq!(params(ended), job_params);
    let trigger_params = [
        ("name", "nightly"),
        ("pipeline", "build"),
        ("schedule", "0 4 * * *"),
        ("trigger_kind", "cron"),
    ];
    assert_eq!(params(&rows[5]), trigger_params);
    assert_eq!(
        (rows[3].area.as_str(), text(&rows[3].subject_id)),
        ("triggers", trigger.as_str())
    );
    assert_eq!(params(&rows[0]), [("name", "TOKEN")]);
    assert_eq!(
        (rows[0].area.as_str(), text(&rows[0].subject_id)),
        ("secrets", secret.as_str())
    );
    assert!(rows.iter().all(|row| row.at.is_some()));
    assert!(rows
        .iter()
        .all(|row| row.params.values().all(|value| !value.contains("hidden"))));
}

/// A delivery whose body has no value for an input makes no job, and records
/// `trigger.fire_failed` with the reason and no actor.
#[sqlx::test(migrations = false)]
async fn list_activity_shows_a_failed_fire(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = Place::new(&app).await;
    let inputs = vec![pointer_input("SHA", "/after")];
    let created = app
        .create_trigger(place.owner, &place.pipeline, "push", None, inputs)
        .await
        .unwrap();
    let (trigger, secret) = (created.trigger.unwrap(), created.webhook_secret);
    let Some(trigger::Kind::Webhook(webhook)) = trigger.kind else {
        panic!("a webhook")
    };

    // when
    let missing = post(&app, &webhook.path, r#"{"ref":"x"}"#, &secret, "delivery-1").await;
    let rows = app.activity_of(place.owner, &place.workspace).await;

    // then
    assert_eq!(missing.0, 422);
    assert_eq!(kinds(&rows), ["trigger.fire_failed", "trigger.created"]);
    let failed = &rows[0];
    assert!(failed.actor_id.is_none() && failed.actor_name.is_empty());
    assert_eq!(failed.params["name"], "push");
    assert_eq!(failed.params["trigger_kind"], "webhook");
    assert!(failed.params["error"].contains("/after"), "{failed:?}");
}

/// An accepted invitation, a role given and a role taken back give one row each, with the role,
/// the person and the actor of the change.
#[sqlx::test(migrations = false)]
async fn list_activity_shows_the_changes_of_the_members(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = Place::new(&app).await;
    let owner = place.owner;
    let workspace = app.workspace_of(owner).await;
    let email = "frank@example.com";
    let invitation = app
        .create_invitation(owner, workspace, email, "member")
        .await
        .unwrap();
    let frank = app.user_with(email, None).await;
    let role = Role::on("project", &place.project, "viewer");

    // when
    app.accept_invitation_by_id(frank, text(&invitation.id))
        .await
        .unwrap();
    app.grant_role(owner, role, &frank.to_string())
        .await
        .unwrap();
    app.revoke_role(owner, role, &frank.to_string())
        .await
        .unwrap();
    let rows = app.activity_of(owner, &place.workspace).await;

    // then
    let all = [
        "member.role_revoked",
        "member.role_granted",
        "member.joined",
    ];
    assert_eq!(kinds(&rows), all);
    assert!(rows.iter().all(|row| row.area == "members"));
    assert!(rows
        .iter()
        .all(|row| text(&row.subject_id) == frank.to_string()));
    let joined = &rows[2];
    assert_eq!(text(&joined.actor_id), frank.to_string());
    assert!(joined.project_id.is_none());
    assert_eq!(
        params(joined),
        [("inviter", "Test User"), ("role", "member")]
    );
    let granted = &rows[1];
    assert_eq!(text(&granted.actor_id), owner.to_string());
    assert_eq!(text(&granted.project_id), place.project);
    assert_eq!(granted.subject_kind, "user");
    assert_eq!(params(granted), [("name", "Test User"), ("role", "viewer")]);
}
