//! The tests of an invitation in the inbox: accept by id, decline, revoke, resend and the end of
//! its validity.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::notifications::v1::notification_invitation::State;
use scylla_testkit::{text, TestApp};
use sqlx::PgPool;
use tonic::Code;

use super::{kinds_of, row_of};
use crate::identity::accounts::assert_refused;
use crate::{app, assert_code};

/// A workspace, its owner, an invited user, and the id of the invitation.
struct Invited {
    /// The owner of the workspace.
    owner: ActorId,
    /// The invited user.
    user: ActorId,
    /// The workspace.
    workspace: WorkspaceId,
    /// The id of the invitation.
    invitation: String,
}

/// Invites a new user with a verified email.
async fn invited(app: &TestApp) -> Invited {
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let email = format!("{}@example.com", ActorId::generate());
    let user = app.user_with(&email, None).await;
    let invitation = app
        .create_invitation(owner, workspace, &email, "member")
        .await
        .unwrap();
    Invited {
        owner,
        user,
        workspace,
        invitation: text(&invitation.id).to_owned(),
    }
}

/// An accept by id gives the role, closes the row, and tells the inviter. The row stays with the
/// state of the invitation, and no longer waits for an action.
#[sqlx::test(migrations = false)]
async fn an_accept_by_id_joins_and_closes_the_row(pool: PgPool) {
    // given
    let app = app(pool).await;
    let invited = invited(&app).await;
    app.dispatch_notifications().await;

    // when
    let joined = app
        .accept_invitation_by_id(invited.user, &invited.invitation)
        .await
        .unwrap();
    let row = row_of(&app, invited.user, "invitation.received").await;
    let inviter = row_of(&app, invited.owner, "invitation.accepted").await;

    // then
    assert_eq!(joined, invited.workspace.to_string());
    assert!(!row.needs_action);
    assert!(row.read_at.is_some());
    let state = row.invitation.and_then(|invitation| invitation.state);
    assert!(matches!(state, Some(State::Accepted(_))));
    assert_eq!(inviter.actor_name, "Test User");
    assert_eq!(inviter.reason, "inviter");
    let summary = app.notification_summary(invited.user).await.unwrap();
    assert_eq!(summary.needs_action_count, 0);
    assert_eq!(summary.unread_count, 0);
}

/// A decline deletes the invitation and the row, and tells the inviter.
#[sqlx::test(migrations = false)]
async fn a_decline_tells_the_inviter_and_deletes_the_invitation(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let invited = invited(&app).await;
    app.dispatch_notifications().await;

    // when
    app.decline_invitation(invited.user, &invited.invitation)
        .await
        .unwrap();
    let second = app
        .decline_invitation(invited.user, &invited.invitation)
        .await;
    let accept = app
        .accept_invitation_by_id(invited.user, &invited.invitation)
        .await;

    // then
    assert_eq!(app.inbox(invited.user).await.len(), 0);
    assert_eq!(kinds_of(&app, invited.owner).await, ["invitation.declined"]);
    assert_code(second, Code::NotFound);
    assert_code(accept, Code::NotFound);
    let left = sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM invitations"#)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

/// A user with no verified email of the invitation cannot answer it.
#[sqlx::test(migrations = false)]
async fn another_user_cannot_answer_an_invitation(pool: PgPool) {
    // given
    let app = app(pool).await;
    let invited = invited(&app).await;
    let other = app.user().await;

    // when
    let accept = app
        .accept_invitation_by_id(other, &invited.invitation)
        .await;
    let decline = app.decline_invitation(other, &invited.invitation).await;

    // then
    assert_refused(accept, "INVITATION_FOR_ANOTHER_EMAIL");
    assert_refused(decline, "INVITATION_FOR_ANOTHER_EMAIL");
}

/// A revoke removes the row, and a fact that waits makes no row.
#[sqlx::test(migrations = false)]
async fn a_revoke_removes_the_row(pool: PgPool) {
    // given
    let app = app(pool).await;
    let first = invited(&app).await;
    app.dispatch_notifications().await;
    let email = format!("{}@example.com", ActorId::generate());
    let waits = app.user_with(&email, None).await;
    let second = app
        .create_invitation(first.owner, first.workspace, &email, "member")
        .await
        .unwrap();

    // when
    let workspace = first.workspace.to_string();
    app.revoke_invitation(first.owner, &workspace, &first.invitation)
        .await
        .unwrap();
    app.revoke_invitation(first.owner, &workspace, text(&second.id))
        .await
        .unwrap();

    // then
    assert_eq!(app.inbox(first.user).await.len(), 0);
    assert_eq!(app.inbox(waits).await.len(), 0);
}

/// The end of the validity closes the row at the purge, and a resend opens it again.
#[sqlx::test(migrations = false)]
async fn a_resend_opens_a_row_that_the_end_of_the_validity_closed(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let invited = invited(&app).await;
    app.dispatch_notifications().await;
    let id = scylla_domain::tenancy::invitations::InvitationId::parse(&invited.invitation).unwrap();
    sqlx::query!(
        "UPDATE invitations SET expires_at = now() - interval '1 day',
             sent_at = now() - interval '1 day' WHERE id = $1",
        id.to_db()
    )
    .execute(&pool)
    .await
    .unwrap();

    // when
    let retention = std::time::Duration::from_secs(90 * 24 * 60 * 60);
    scylla_core::features::notifications::purge::run(&pool, retention)
        .await
        .unwrap();
    let ended = row_of(&app, invited.user, "invitation.received").await;
    let workspace = invited.workspace.to_string();
    app.resend_invitation(invited.owner, &workspace, &invited.invitation)
        .await
        .unwrap();
    let open = row_of(&app, invited.user, "invitation.received").await;

    // then
    assert!(!ended.needs_action);
    let state = ended.invitation.and_then(|invitation| invitation.state);
    assert!(matches!(state, Some(State::Expired(_))));
    assert!(open.needs_action);
    let state = open.invitation.and_then(|invitation| invitation.state);
    assert!(matches!(state, Some(State::Open(_))));
}
