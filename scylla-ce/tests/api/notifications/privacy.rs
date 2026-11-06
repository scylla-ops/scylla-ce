//! The tests of the privacy of a row: who can read it, what it holds, and what the inviter learns.

use prost::Message;
use prost_types::FileDescriptorSet;
use scylla_domain::identity::actors::ActorId;
use scylla_proto::notifications::v1::ListNotificationsRequest;
use scylla_proto::FILE_DESCRIPTOR_SET;
use scylla_testkit::{text, with_token, Role, TestApp};
use sqlx::PgPool;
use tonic::Code;
use uuid::Uuid;

use super::{kinds_of, row_id};
use crate::identity::accounts::PASSWORD;
use crate::{app, assert_code};

/// The inviter gets the same answer for a known and for an unknown email.
#[sqlx::test(migrations = false)]
async fn the_answer_to_the_inviter_is_the_same_for_a_known_and_an_unknown_email(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    app.user_with("known@example.com", None).await;

    // when
    let known = app
        .create_invitation(owner, workspace, "known@example.com", "viewer")
        .await
        .unwrap();
    let unknown = app
        .create_invitation(owner, workspace, "unknown@example.com", "viewer")
        .await
        .unwrap();

    // then
    assert_eq!(known.role, unknown.role);
    assert_eq!(known.workspace_id, unknown.workspace_id);
    assert_eq!(known.invited_by, unknown.invited_by);
    assert_ne!(known.id, unknown.id);
}

/// An inviter that lost its role does not see the row about the answer, and the row goes.
#[sqlx::test(migrations = false)]
async fn a_revoked_person_does_not_see_the_row(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (admin, invitee) = (app.user().await, "ann@example.com");
    let id = workspace.to_string();
    let admin_role = Role::on("workspace", &id, "admin");
    app.grant_role(owner, admin_role, &admin.to_string())
        .await
        .unwrap();
    let user = app.user_with(invitee, None).await;
    let invitation = app
        .create_invitation(admin, workspace, invitee, "viewer")
        .await
        .unwrap();
    app.accept_invitation_by_id(user, text(&invitation.id))
        .await
        .unwrap();
    let before = kinds_of(&app, admin).await;
    let accepted = before.iter().any(|kind| kind == "invitation.accepted");

    // when
    app.revoke_role(owner, admin_role, &admin.to_string())
        .await
        .unwrap();
    let after = kinds_of(&app, admin).await;

    // then
    assert!(accepted, "{before:?}");
    assert!(!after.iter().any(|kind| kind == "invitation.accepted"));
    let left = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM notifications
           WHERE recipient_id = $1 AND kind = 'invitation.accepted'"#,
        admin.to_db()
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, 0);
}

/// A gate that fails hides a row with an access rule, and keeps it.
#[sqlx::test(migrations = false)]
async fn a_gate_error_hides_the_row_and_keeps_it(pool: PgPool) {
    // given
    let app = TestApp::spawn_without_spicedb(pool.clone(), scylla_ce::edition()).await;
    let owner = app.user().await;
    let reader = app.user().await;
    let workspace = app.workspace_of(owner).await;
    sqlx::query!(
        "INSERT INTO notifications (id, recipient_id, kind, reason, group_key, workspace_id)
         VALUES ($1, $2, 'invitation.accepted', 'inviter', 'g', $3)",
        Uuid::now_v7(),
        reader.to_db(),
        workspace.to_db(),
    )
    .execute(&pool)
    .await
    .unwrap();

    // when
    let inbox = app.inbox(reader).await;

    // then
    assert_eq!(inbox.len(), 0);
    let left = sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM notifications"#)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 1);
}

/// A person that lost its last role keeps the row about it, with the snapshot of the name.
#[sqlx::test(migrations = false)]
async fn a_lost_role_keeps_the_own_row(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let user = app.user().await;
    let id = workspace.to_string();
    let viewer = Role::on("workspace", &id, "viewer");
    app.grant_role(owner, viewer, &user.to_string())
        .await
        .unwrap();
    app.revoke_role(owner, viewer, &user.to_string())
        .await
        .unwrap();
    sqlx::query!(
        "UPDATE workspaces SET name = 'Renamed' WHERE id = $1",
        workspace.to_db()
    )
    .execute(&pool)
    .await
    .unwrap();

    // when
    let inbox = app.inbox(user).await;

    // then
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].reason, "subject");
    assert_eq!(inbox[0].workspace_name, "Personal");
}

/// A row of another person gives `NOT_FOUND` on each route that takes an id, and nothing changes.
#[sqlx::test(migrations = false)]
async fn a_row_of_another_person_gives_not_found(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (ana, bob) = (app.user().await, app.user().await);
    app.create_personal_token(ana, "ana", None).await.unwrap();
    app.create_personal_token(bob, "bob", None).await.unwrap();
    let ana_rows = app.inbox(ana).await;
    let bob_rows = app.inbox(bob).await;
    let (hers, his) = (row_id(&ana_rows[0]), row_id(&bob_rows[0]));

    // when
    let read = app.mark_notifications_read(ana, &[hers, his], true).await;
    let done = app.set_notifications_done(ana, &[hers, his], true).await;

    // then
    assert_code(read, Code::NotFound);
    assert_code(done, Code::NotFound);
    let inbox = app.inbox(ana).await;
    assert!(inbox[0].read_at.is_none());
    assert_eq!(inbox.len(), 1);
}

/// No request of the service has a field that names a recipient.
#[test]
fn no_route_takes_a_recipient() {
    // given
    let set = FileDescriptorSet::decode(FILE_DESCRIPTOR_SET).unwrap();
    let file = set
        .file
        .iter()
        .find(|file| file.package() == "scylla.notifications.v1")
        .expect("the file of the service");

    // when
    let fields: Vec<&str> = file
        .message_type
        .iter()
        .filter(|message| message.name().ends_with("Request"))
        .flat_map(|message| message.field.iter().map(|field| field.name()))
        .collect();

    // then
    assert!(!fields.is_empty());
    for field in fields {
        assert!(!field.contains("user"), "{field}");
        assert!(!field.contains("recipient"), "{field}");
        assert!(!field.contains("actor"), "{field}");
    }
}

/// An actor that is not a user has no inbox.
#[sqlx::test(migrations = false)]
async fn an_app_cannot_call_the_routes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (_key, token) = app.app_in(owner, workspace, false).await;
    let summary = scylla_proto::notifications::v1::GetNotificationSummaryRequest {};
    let list = ListNotificationsRequest::default();

    // when
    let summary = app
        .notifications()
        .get_notification_summary(with_token(&token, summary))
        .await;
    let list = app
        .notifications()
        .list_notifications(with_token(&token, list))
        .await;

    // then
    assert_code(summary, Code::PermissionDenied);
    assert_code(list, Code::PermissionDenied);
}

/// A token from an exchange of a personal token has no inbox, and a session has one.
#[sqlx::test(migrations = false)]
async fn an_api_token_cannot_call_the_routes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "api@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await;
    let created = app.create_personal_token(user, "ci", None).await.unwrap();
    let exchanged = app.exchange_api_token(&created.secret).await.unwrap();
    let session = app.sign_in(email, PASSWORD).await.unwrap();
    let signed_in = session.token.unwrap().value;
    let request = scylla_proto::notifications::v1::GetNotificationSummaryRequest {};

    // when
    let by_token = app
        .notifications()
        .get_notification_summary(with_token(&exchanged, request))
        .await;
    let by_session = app
        .notifications()
        .get_notification_summary(with_token(&signed_in, request))
        .await;

    // then
    assert_code(by_token, Code::PermissionDenied);
    assert!(by_session.is_ok(), "{by_session:?}");
}

/// A row holds no email of another person: the row of the inviter names the actor by its id.
#[sqlx::test(migrations = false)]
async fn a_row_holds_no_email(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let email = "private@example.com";
    let user: ActorId = app.user_with(email, None).await;
    let invitation = app
        .create_invitation(owner, workspace, email, "viewer")
        .await
        .unwrap();
    app.dispatch_notifications().await;
    app.accept_invitation_by_id(user, text(&invitation.id))
        .await
        .unwrap();
    app.dispatch_notifications().await;

    // when
    let rows = sqlx::query_scalar!(
        r#"SELECT params::text AS "params!" FROM notifications
           UNION ALL SELECT params::text AS "params!" FROM notification_events"#
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    // then
    assert!(!rows.is_empty());
    assert!(
        rows.iter().all(|params| !params.contains(email)),
        "{rows:?}"
    );
}
