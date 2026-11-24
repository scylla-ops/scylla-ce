//! The tests of the hooks of a user: an edition follows the deletion of a user, and adds its part
//! to the export of the data of a user.

use std::sync::{Arc, Mutex};

use scylla_domain::identity::actors::ActorId;
use scylla_extension::{
    async_trait, AfterDeleteUser, ExportPart, ExportUser, HookError, Rejection, Tx, UserContext,
};
use scylla_testkit::TestApp;
use serde_json::{json, Value};
use sqlx::PgPool;
use tonic::Code;

use crate::assert_code;
use crate::identity::accounts::PASSWORD;

/// The log of the `Record` hook: the user and the actor of each call.
type Log = Arc<Mutex<Vec<(ActorId, ActorId)>>>;

/// A hook that records each call, and refuses a deletion if `refuse` is set.
struct Record {
    /// The shared log.
    log: Log,
    /// Refuse each deletion after the record.
    refuse: bool,
}

#[async_trait]
impl AfterDeleteUser for Record {
    async fn after_delete_user(
        &self,
        _tx: &mut Tx<'_>,
        ctx: &UserContext,
    ) -> Result<(), HookError> {
        self.log.lock().unwrap().push((ctx.user, ctx.actor));
        if self.refuse {
            return Err(Rejection::new("TEST_REFUSED", "refused by the test hook").into());
        }
        Ok(())
    }
}

#[async_trait]
impl ExportUser for Record {
    async fn export_user(
        &self,
        _tx: &mut Tx<'_>,
        ctx: &UserContext,
    ) -> Result<ExportPart, HookError> {
        self.log.lock().unwrap().push((ctx.user, ctx.actor));
        Ok(ExportPart::new("notes", json!([{ "text": "a note" }])))
    }
}

/// Starts a server whose edition has a `Record` hook for the deletion and the export.
async fn app_with(pool: PgPool, refuse: bool) -> (TestApp, Log) {
    let log = Log::default();
    let hook = Arc::new(Record {
        log: Arc::clone(&log),
        refuse,
    });
    let edition = scylla_core::edition("test")
        .after_delete_user(hook.clone())
        .export_user(hook)
        .build();
    (TestApp::spawn(pool, edition).await, log)
}

/// Makes a user with a password, and returns it with its access token.
async fn signed_in(app: &TestApp, email: &str) -> (ActorId, String) {
    let user = app.user_with(email, Some(PASSWORD)).await;
    let session = app.sign_in(email, PASSWORD).await.unwrap();
    (user, session.token.unwrap().value)
}

/// `DeleteAccount` runs the hooks of a user with the user as the actor, and `DeleteUser` with
/// the instance admin as the actor.
#[sqlx::test(migrations = false)]
async fn delete_account_runs_the_hooks_of_a_user(pool: PgPool) {
    // given
    let (app, log) = app_with(pool, false).await;
    let (user, token) = signed_in(&app, "ada@example.com").await;
    let other = app.user().await;
    let admin = app.admin().await;

    // when
    app.delete_account(&token, PASSWORD).await.unwrap();
    app.delete_user(admin, &other.to_string()).await.unwrap();

    // then
    assert_eq!(*log.lock().unwrap(), [(user, user), (other, admin)]);
}

/// A hook that refuses rolls the deletion back: the account stays.
#[sqlx::test(migrations = false)]
async fn delete_account_keeps_the_user_when_a_hook_refuses(pool: PgPool) {
    // given
    let (app, log) = app_with(pool, true).await;
    let (user, token) = signed_in(&app, "ada@example.com").await;

    // when
    let result = app.delete_account(&token, PASSWORD).await;

    // then
    assert_code(result, Code::FailedPrecondition);
    assert_eq!(*log.lock().unwrap(), [(user, user)]);
    let me = app.get_me(&token).await.unwrap();
    assert_eq!(me.id.unwrap().value, user.to_string());
}

/// The export holds the part of each hook under `edition`, by its name.
#[sqlx::test(migrations = false)]
async fn export_account_data_adds_the_parts_of_the_edition(pool: PgPool) {
    // given
    let (app, log) = app_with(pool, false).await;
    let (user, token) = signed_in(&app, "ada@example.com").await;

    // when
    let export = app.export_account_data(&token).await.unwrap();

    // then
    let data: Value = serde_json::from_str(&export.json).unwrap();
    assert_eq!(data["edition"]["notes"][0]["text"], "a note");
    assert_eq!(*log.lock().unwrap(), [(user, user)]);
}
