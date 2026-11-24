//! The tests of an acceptance at the same time as a revoke or a resend: the first one wins, and
//! a stopped link gives no role.

use scylla_domain::authz::Check;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_testkit::{text, TestApp};
use sqlx::PgPool;
use tonic::{Code, Status};

use super::open_invitations::age;
use crate::app;

/// The invitations of the rounds: the user, its token, the link, and the invitation id.
type Round = (ActorId, String, String, String);

/// Invites ten new users into the workspace of `owner`, with a cooldown that is over.
async fn rounds(app: &TestApp, owner: ActorId) -> Vec<Round> {
    let workspace = app.workspace_of(owner).await;
    let mut rounds = Vec::new();
    for round in 0..10 {
        let email = format!("race{round}@example.com");
        let invitation = app
            .create_invitation(owner, workspace, &email, "viewer")
            .await;
        let id = text(&invitation.unwrap().id).to_owned();
        age(app, &id, false).await;
        let link = app.mails.token(&email, "accept-invitation");
        let user = app.user_with(&email, None).await;
        rounds.push((user, app.token(user).await, link, id));
    }
    rounds
}

/// Checks the result of one round: the acceptance wins and the other route finds no open
/// invitation, or the other route wins and the link gives no role.
async fn check(
    app: &TestApp,
    user: ActorId,
    accepted: Result<String, Status>,
    other: bool,
    workspace: WorkspaceId,
) {
    let reads: Check = Workspace::GET_WORKSPACE.on(workspace);
    match (accepted, other) {
        (Ok(_), false) => assert!(app.can(user, reads).await),
        (Err(status), true) => {
            assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
            assert!(!app.can(user, reads).await);
        }
        (accepted, other) => panic!("one of the two must win: {accepted:?} {other}"),
    }
}

/// An acceptance and a revoke at the same time never both pass, and a revoked link gives no
/// role. Ten rounds, so that a race shows.
#[sqlx::test(migrations = false)]
async fn an_accept_and_a_revoke_never_both_pass(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let rounds = rounds(&app, owner).await;
    let place = workspace.to_string();

    // when
    let mut results = Vec::new();
    for (user, token, link, id) in &rounds {
        let (accepted, revoked) = tokio::join!(
            app.accept_invitation(token, link),
            app.revoke_invitation(owner, &place, id),
        );
        if let Err(status) = &revoked {
            assert_eq!(status.code(), Code::NotFound, "{status:?}");
        }
        results.push((*user, accepted, revoked.is_ok()));
    }

    // then
    for (user, accepted, revoked) in results {
        check(&app, user, accepted, revoked, workspace).await;
    }
}

/// An acceptance with the old link and a resend at the same time never both pass, and a link
/// that a resend stopped gives no role. Ten rounds, so that a race shows.
#[sqlx::test(migrations = false)]
async fn an_accept_and_a_resend_never_both_pass(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let rounds = rounds(&app, owner).await;
    let place = workspace.to_string();

    // when
    let mut results = Vec::new();
    for (user, token, link, id) in &rounds {
        let (accepted, resent) = tokio::join!(
            app.accept_invitation(token, link),
            app.resend_invitation(owner, &place, id),
        );
        if let Err(status) = &resent {
            assert_eq!(status.code(), Code::NotFound, "{status:?}");
        }
        results.push((*user, accepted, resent.is_ok()));
    }

    // then
    for (user, accepted, resent) in results {
        check(&app, user, accepted, resent, workspace).await;
    }
}
