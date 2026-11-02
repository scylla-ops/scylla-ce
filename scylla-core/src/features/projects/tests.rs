//! Unit tests of the projects service: the hook behavior that the API cannot show.

use std::sync::{Arc, Mutex};

use scylla_domain::actors::{Actor, ActorId, ActorKind};
use scylla_domain::projects::{NewProject, ProjectName, KIND};
use scylla_domain::ResourceKind;
use scylla_extension::{
    async_trait, AfterDelete, BeforeCreate, CreateContext, DeleteContext, Edition, EditionBuilder,
    HookError, Hooks, Rejection, Tx,
};
use sqlx::PgPool;

use super::service;
use crate::features::actors;
use crate::Error;

/// The log of the `Record` hooks: hook name, kind, actor and owner of each call.
type Log = Arc<Mutex<Vec<(&'static str, ResourceKind, ActorId, ActorId)>>>;

/// A hook that inserts an actor, then refuses.
struct WriteThenReject;

#[async_trait]
impl BeforeCreate for WriteThenReject {
    async fn before_create(&self, tx: &mut Tx<'_>, _ctx: &CreateContext) -> Result<(), HookError> {
        sqlx::query!(
            "INSERT INTO actors (id, kind) VALUES ($1, 'written by the hook')",
            ActorId::generate().to_db(),
        )
        .execute(&mut **tx)
        .await?;
        Err(Rejection::new("TEST_REJECTED", "rejected by the test hook").into())
    }
}

/// A hook that records each call, and refuses if `reject` is set.
struct Record {
    /// The name in the log.
    name: &'static str,
    /// The shared log.
    log: Log,
    /// Refuse each call after the record.
    reject: bool,
}

impl Record {
    /// Adds the call to the log, then accepts or refuses it.
    fn record(
        &self,
        kind: ResourceKind,
        actor: &Actor,
        owner_id: ActorId,
    ) -> Result<(), HookError> {
        let call = (self.name, kind, actor.id, owner_id);
        self.log.lock().unwrap().push(call);
        if self.reject {
            return Err(Rejection::new("TEST_REJECTED", "rejected by the test hook").into());
        }
        Ok(())
    }
}

#[async_trait]
impl BeforeCreate for Record {
    async fn before_create(&self, _tx: &mut Tx<'_>, ctx: &CreateContext) -> Result<(), HookError> {
        self.record(ctx.kind, &ctx.actor, ctx.owner_id)
    }
}

#[async_trait]
impl AfterDelete for Record {
    async fn after_delete(&self, _tx: &mut Tx<'_>, ctx: &DeleteContext) -> Result<(), HookError> {
        self.record(ctx.kind, &ctx.actor, ctx.owner_id)
    }
}

/// Makes a `Record` hook on `log`.
fn record(name: &'static str, log: &Log, reject: bool) -> Arc<Record> {
    Arc::new(Record {
        name,
        log: Arc::clone(log),
        reject,
    })
}

/// Creates an actor.
async fn actor(pool: &PgPool) -> Actor {
    actors::create(pool, ActorKind::new("test")).await.unwrap()
}

/// Makes the input of a creation by `author` for `owner_id`.
fn new_project(author: &Actor, owner_id: ActorId) -> NewProject {
    NewProject {
        owner_id,
        created_by: author.id,
        name: ProjectName::parse("alpha".to_owned()).unwrap(),
    }
}

/// Counts the actors.
async fn actor_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM actors"#)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Returns the hooks of a test edition.
fn hooks(edition: EditionBuilder) -> Arc<Hooks> {
    edition.build().hooks()
}

/// A refusal also removes what the hook wrote before it.
#[sqlx::test(migrator = "crate::db::MIGRATOR")]
async fn create_rolls_back_the_hook_writes_on_a_refusal(pool: PgPool) {
    // given
    let author = actor(&pool).await;
    let hooks = hooks(Edition::builder("test").before_create(KIND, Arc::new(WriteThenReject)));

    // when
    let new = new_project(&author, author.id);
    let result = service::create(&pool, &hooks, author, new).await;

    // then
    assert!(matches!(result, Err(Error::Rejected(_))));
    assert_eq!(actor_count(&pool).await, 1);
}

/// The hooks get the kind, the actor and the owner, on create and on delete.
#[sqlx::test(migrator = "crate::db::MIGRATOR")]
async fn hooks_get_the_kind_the_actor_and_the_owner(pool: PgPool) {
    // given
    let author = actor(&pool).await;
    let owner = actor(&pool).await;
    let log = Log::default();
    let hook = record("hook", &log, false);
    let hooks = hooks(
        Edition::builder("test")
            .before_create(KIND, hook.clone())
            .after_delete(KIND, hook),
    );

    // when
    let new = new_project(&author, owner.id);
    let project = service::create(&pool, &hooks, author.clone(), new)
        .await
        .unwrap();
    service::delete(&pool, &hooks, author.clone(), project.id)
        .await
        .unwrap();

    // then
    let call = ("hook", KIND, author.id, owner.id);
    assert_eq!(*log.lock().unwrap(), vec![call, call]);
}

/// The hooks run in registration order, and the first refusal stops the next ones.
#[sqlx::test(migrator = "crate::db::MIGRATOR")]
async fn hooks_run_in_order_and_stop_at_the_first_refusal(pool: PgPool) {
    // given
    let author = actor(&pool).await;
    let log = Log::default();
    let hooks = hooks(
        Edition::builder("test")
            .before_create(KIND, record("first", &log, false))
            .before_create(KIND, record("second", &log, true))
            .before_create(KIND, record("third", &log, false)),
    );

    // when
    let new = new_project(&author, author.id);
    let result = service::create(&pool, &hooks, author, new).await;

    // then
    assert!(matches!(result, Err(Error::Rejected(_))));
    let names: Vec<_> = log.lock().unwrap().iter().map(|call| call.0).collect();
    assert_eq!(names, vec!["first", "second"]);
}

/// A hook of another kind does not run for a project.
#[sqlx::test(migrator = "crate::db::MIGRATOR")]
async fn hooks_of_another_kind_do_not_run(pool: PgPool) {
    // given
    let author = actor(&pool).await;
    let log = Log::default();
    let other = ResourceKind::new("other");
    let hooks = hooks(Edition::builder("test").before_create(other, record("other", &log, true)));

    // when
    let new = new_project(&author, author.id);
    let result = service::create(&pool, &hooks, author, new).await;

    // then
    assert!(result.is_ok());
    assert!(log.lock().unwrap().is_empty());
}
