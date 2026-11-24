//! The tests of the open invitations of a workspace: the list, the revoke, and the resend with
//! its cooldown.

use std::time::{SystemTime, UNIX_EPOCH};

use scylla_domain::tenancy::invitations::InvitationId;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_testkit::{text, TestApp};
use sqlx::PgPool;
use tonic::Code;
use tonic_types::StatusExt;

use crate::identity::accounts::assert_refused;
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// Moves a time of an invitation two minutes back: the last mail, or the end of the link.
pub(super) async fn age(app: &TestApp, invitation: &str, expire: bool) {
    let id = InvitationId::parse(invitation).unwrap().to_db();
    let sql = if expire {
        "UPDATE invitations SET expires_at = now() - interval '2 minutes' WHERE id = $1"
    } else {
        "UPDATE invitations SET sent_at = now() - interval '2 minutes' WHERE id = $1"
    };
    sqlx::query(sql).bind(id).execute(&app.pool).await.unwrap();
}

/// An admin lists the invitations that nobody accepted, oldest first, expired ones too. A member
/// and an unknown workspace get a refusal.
#[sqlx::test(migrations = false)]
async fn list_invitations_shows_the_open_ones(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, member) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::MEMBER.of(workspace, member)]).await;
    let mut created = Vec::new();
    for email in ["a@example.com", "b@example.com", "c@example.com"] {
        let invitation = app
            .create_invitation(owner, workspace, email, "viewer")
            .await;
        created.push(invitation.unwrap());
    }
    let joiner = app.user_with("c@example.com", None).await;
    let link = app.mails.token("c@example.com", "accept-invitation");
    app.accept_invitation(&app.token(joiner).await, &link)
        .await
        .unwrap();
    age(&app, text(&created[1].id), true).await;
    let id = workspace.to_string();

    // when
    let listed = app.list_invitations(owner, &id).await.unwrap();
    let by_member = app.list_invitations(member, &id).await;
    let unknown = app.list_invitations(owner, UNKNOWN_ID).await;

    // then
    let emails: Vec<_> = listed.iter().map(|i| i.email.as_str()).collect();
    assert_eq!(emails, ["a@example.com", "b@example.com"]);
    assert_eq!(text(&listed[0].invited_by), owner.to_string());
    assert_code(by_member, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
}

/// A revoked invitation leaves the list and its link stops. A second revoke, an invitation of
/// another workspace and an accepted invitation give `NOT_FOUND`. A member gets a refusal.
#[sqlx::test(migrations = false)]
async fn revoke_invitation_stops_the_link(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, other, member) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::MEMBER.of(workspace, member)]).await;
    let email = "dan@example.com";
    let invitation = app
        .create_invitation(owner, workspace, email, "member")
        .await;
    let invitation = invitation.unwrap();
    let link = app.mails.token(email, "accept-invitation");
    let dan = app.user_with(email, None).await;
    let (id, invitation_id) = (workspace.to_string(), text(&invitation.id).to_owned());
    let elsewhere = app.workspace_of(other).await.to_string();

    // when
    let by_member = app.revoke_invitation(member, &id, &invitation_id).await;
    let wrong_workspace = app
        .revoke_invitation(other, &elsewhere, &invitation_id)
        .await;
    app.revoke_invitation(owner, &id, &invitation_id)
        .await
        .unwrap();
    let again = app.revoke_invitation(owner, &id, &invitation_id).await;
    let accepted = app.accept_invitation(&app.token(dan).await, &link).await;
    let bad_id = app.revoke_invitation(owner, &id, "nope").await;

    // then
    assert_code(by_member, Code::PermissionDenied);
    assert_code(wrong_workspace, Code::NotFound);
    assert_code(again, Code::NotFound);
    assert_invalid_field(&accepted.unwrap_err(), "token");
    assert_invalid_field(&bad_id.unwrap_err(), "invitation_id");
    assert!(app.list_invitations(owner, &id).await.unwrap().is_empty());
}

/// A resend gives a new link valid 7 days, and the old link stops. A second resend in less than
/// 60 seconds gives `RESEND_TOO_SOON` with the seconds to wait, and an accepted invitation
/// gives `NOT_FOUND`.
#[sqlx::test(migrations = false)]
async fn resend_invitation_sends_a_new_link(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, member) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::MEMBER.of(workspace, member)]).await;
    let email = "fay@example.com";
    let invitation = app
        .create_invitation(owner, workspace, email, "member")
        .await;
    let (id, invitation_id) = (
        workspace.to_string(),
        text(&invitation.unwrap().id).to_owned(),
    );
    let old_link = app.mails.token(email, "accept-invitation");
    let fay = app.user_with(email, None).await;

    // when
    let too_soon = app.resend_invitation(owner, &id, &invitation_id).await;
    age(&app, &invitation_id, false).await;
    let by_member = app.resend_invitation(member, &id, &invitation_id).await;
    let resent = app.resend_invitation(owner, &id, &invitation_id).await;
    let new_link = app.mails.token(email, "accept-invitation");
    let again = app.resend_invitation(owner, &id, &invitation_id).await;
    let token = app.token(fay).await;
    let with_old = app.accept_invitation(&token, &old_link).await;
    let with_new = app.accept_invitation(&token, &new_link).await;
    let after_accept = app.resend_invitation(owner, &id, &invitation_id).await;

    // then
    let status = too_soon.unwrap_err();
    let info = status.get_details_error_info().expect("ErrorInfo detail");
    assert_eq!(info.reason, "RESEND_TOO_SOON");
    let wait: i64 = info.metadata["retry_after_seconds"].parse().unwrap();
    assert!((1..=60).contains(&wait), "{wait}");
    assert_code(by_member, Code::PermissionDenied);
    let resent = resent.unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let expires = resent.expires_at.unwrap().seconds - now;
    assert!(
        (7 * 86_400 - 60..=7 * 86_400).contains(&expires),
        "{expires}"
    );
    assert_eq!(app.mails.to(email).len(), 2);
    assert_ne!(old_link, new_link);
    assert_refused(again, "RESEND_TOO_SOON");
    assert_invalid_field(&with_old.unwrap_err(), "token");
    assert_eq!(with_new.unwrap(), id);
    assert_code(after_accept, Code::NotFound);
}

/// A second invitation of an email that has an open invitation in the workspace gives
/// `INVITATION_PENDING`, expired or not. Another workspace and a revoked invitation do not
/// count.
#[sqlx::test(migrations = false)]
async fn create_invitation_refuses_a_pending_email(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, other) = (app.user().await, app.user().await);
    let (workspace, elsewhere) = (app.workspace_of(owner).await, app.workspace_of(other).await);
    let email = "gus@example.com";
    let first = app
        .create_invitation(owner, workspace, email, "viewer")
        .await;
    let first = text(&first.unwrap().id).to_owned();
    age(&app, &first, true).await;

    // when
    let pending = app
        .create_invitation(owner, workspace, email, "admin")
        .await;
    let in_other = app
        .create_invitation(other, elsewhere, email, "viewer")
        .await;
    app.revoke_invitation(owner, &workspace.to_string(), &first)
        .await
        .unwrap();
    let after_revoke = app
        .create_invitation(owner, workspace, email, "admin")
        .await;

    // then
    assert_refused(pending, "INVITATION_PENDING");
    assert!(in_other.is_ok() && after_revoke.is_ok());
}
