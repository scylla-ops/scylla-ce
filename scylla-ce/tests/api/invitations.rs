//! The tests of `InvitationService`: an admin invites an email, and the person with this email
//! gets the role.

use scylla_domain::actors::ActorId;
use sqlx::PgPool;
use tonic::Code;

use crate::accounts::{assert_refused, PASSWORD};
use crate::{app, assert_code, assert_invalid_field};

/// An admin invites an email. The user with this verified email accepts, one time, and works in
/// the workspace.
#[sqlx::test(migrations = false)]
async fn an_invitation_gives_its_role(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let email = "frank@example.com";
    let invitation = app
        .create_invitation(owner, workspace, email, "member")
        .await
        .unwrap();
    let frank = app.user_with(email, Some(PASSWORD)).await;
    let session = app.sign_in(email, PASSWORD).await.unwrap();
    let token = session.token.unwrap().value;
    let link = app.mails.token(email, "accept-invitation");

    // when
    let joined = app.accept_invitation(&token, &link).await.unwrap();
    let again = app.accept_invitation(&token, &link).await;
    let created = app.create_project_in(frank, workspace, "frank").await;

    // then
    assert_eq!(
        (invitation.email.as_str(), invitation.role.as_str()),
        (email, "member")
    );
    assert_eq!(joined, workspace.to_string());
    assert_invalid_field(&again.unwrap_err(), "token");
    assert!(created.is_ok(), "{created:?}");
}

/// Only an admin invites, with a role of the workspaces and an email.
#[sqlx::test(migrations = false)]
async fn create_invitation_needs_an_admin_and_a_role(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let stranger = app.user().await;
    let workspace = app.workspace_of(owner).await;

    // when
    let by_stranger = app
        .create_invitation(stranger, workspace, "g@example.com", "member")
        .await;
    let bad_role = app
        .create_invitation(owner, workspace, "g@example.com", "owner")
        .await;
    let bad_email = app
        .create_invitation(owner, workspace, "nope", "member")
        .await;

    // then
    assert_code(by_stranger, Code::PermissionDenied);
    assert_invalid_field(&bad_role.unwrap_err(), "role");
    assert_invalid_field(&bad_email.unwrap_err(), "email");
}

/// An invitation is for the person with its email only.
#[sqlx::test(migrations = false)]
async fn accept_invitation_needs_the_email_of_the_invitation(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    app.create_invitation(owner, workspace, "grace@example.com", "viewer")
        .await
        .unwrap();
    let link = app.mails.token("grace@example.com", "accept-invitation");
    app.user_with("mallory@example.com", Some(PASSWORD)).await;
    let session = app.sign_in("mallory@example.com", PASSWORD).await.unwrap();

    // when
    let result = app
        .accept_invitation(&session.token.unwrap().value, &link)
        .await;

    // then
    assert_refused(result, "INVITATION_FOR_ANOTHER_EMAIL");
}

/// In the mode `invite_only`, the link of an invitation makes an active user that works in the
/// workspace. A sign-up with the link of another email is refused.
#[sqlx::test(migrations = false)]
async fn sign_up_with_an_invitation(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let email = "henry@example.com";
    app.create_invitation(owner, workspace, email, "member")
        .await
        .unwrap();
    let link = app.mails.token(email, "accept-invitation");

    // when
    let other_email = app.sign_up("ivy@example.com", PASSWORD, &link).await;
    app.sign_up(email, PASSWORD, &link).await.unwrap();
    let session = app.sign_in(email, PASSWORD).await.unwrap();

    // then
    assert_invalid_field(&other_email.unwrap_err(), "invitation_token");
    let me = app.get_me(&session.token.unwrap().value).await.unwrap();
    let henry = ActorId::parse(&me.id.unwrap().value).unwrap();
    let created = app.create_project_in(henry, workspace, "henry").await;
    assert!(created.is_ok(), "{created:?}");
}
