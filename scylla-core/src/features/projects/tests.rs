//! Unit tests of the projects service: the hook behavior that the API cannot show.

use std::sync::{Arc, Mutex};

use scylla_domain::authz::{Check, Object, Relationship};
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::users::{Email, NewUser};
use scylla_domain::projects::{NewProject, ProjectDescription, ProjectName, KIND};
use scylla_domain::tenancy::slugs::Slug;
use scylla_domain::tenancy::workspaces::{NewSlug, NewWorkspace, WorkspaceId};
use scylla_domain::{Name, ResourceKind};
use scylla_extension::{
    async_trait, AccessError, AfterDelete, BeforeCreate, Binding, CreateContext, DeleteContext,
    Edition, EditionBuilder, Gate, HookError, Hooks, IssuedToken, Operation, Principal, Rejection,
    RolePage, Tx,
};
use sqlx::PgPool;
use tonic::metadata::MetadataMap;

use super::service;
use crate::features::identity::accounts;
use crate::features::tenancy::workspaces;
use crate::Error;

/// The log of the `Record` hooks: hook name, kind, actor and workspace of each call.
type Log = Arc<Mutex<Vec<(&'static str, ResourceKind, ActorId, WorkspaceId)>>>;

/// A gate that allows each check and keeps no relationship: these tests check only the hooks.
struct NoGate;

#[async_trait]
impl Gate for NoGate {
    async fn authenticate(
        &self,
        _metadata: &MetadataMap,
        _operation: &Operation,
    ) -> Result<Principal, AccessError> {
        Err(AccessError::Unauthenticated(
            "no gate in unit tests".to_owned(),
        ))
    }

    async fn authorize(&self, _principal: &Principal, _check: Check) -> Result<(), AccessError> {
        Ok(())
    }

    fn issue(&self, _actor: &Actor, _binding: Binding) -> Result<IssuedToken, AccessError> {
        Err(AccessError::Unavailable("no gate in unit tests".to_owned()))
    }

    async fn write(&self, _relationships: &[Relationship]) -> Result<String, AccessError> {
        Ok(String::new())
    }

    async fn delete(&self, _relationships: &[Relationship]) -> Result<(), AccessError> {
        Ok(())
    }

    async fn delete_object(&self, _object: &Object) -> Result<(), AccessError> {
        Ok(())
    }

    async fn delete_subject(&self, _subject: &Object) -> Result<(), AccessError> {
        Ok(())
    }

    async fn subjects(&self, _object: &Object) -> Result<Vec<ActorId>, AccessError> {
        Ok(Vec::new())
    }

    async fn roles(
        &self,
        _object: &Object,
        _cursor: Option<&str>,
        _count: usize,
    ) -> Result<Option<RolePage>, AccessError> {
        Ok(Some(RolePage::default()))
    }

    async fn permissions(
        &self,
        _principal: &Principal,
        _object: &Object,
        _workspace: Option<WorkspaceId>,
        permissions: &[&'static str],
    ) -> Result<Vec<bool>, AccessError> {
        Ok(vec![true; permissions.len()])
    }

    async fn lookup(
        &self,
        _principal: &Principal,
        _definition: &'static str,
        _permission: &'static str,
    ) -> Result<Vec<String>, AccessError> {
        Ok(Vec::new())
    }
}

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
        workspace_id: WorkspaceId,
    ) -> Result<(), HookError> {
        let call = (self.name, kind, actor.id, workspace_id);
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
        self.record(ctx.kind, &ctx.actor, ctx.workspace_id)
    }
}

#[async_trait]
impl AfterDelete for Record {
    async fn after_delete(&self, _tx: &mut Tx<'_>, ctx: &DeleteContext) -> Result<(), HookError> {
        self.record(ctx.kind, &ctx.actor, ctx.workspace_id)
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

/// Creates a user with its personal workspace.
async fn actor(pool: &PgPool) -> Actor {
    let new = NewUser {
        email: Email::parse(format!("{}@test.example", ActorId::generate())).unwrap(),
        email_verified: true,
        display_name: Name::parse("Test".to_owned()).unwrap(),
        handle: None,
        password: None,
    };
    accounts::create_user(pool, new).await.unwrap()
}

/// Creates a workspace that `owner` owns.
async fn workspace(pool: &PgPool, owner: &Actor) -> WorkspaceId {
    let new = NewWorkspace {
        creator: owner.id,
        name: Name::parse("main".to_owned()).unwrap(),
        slug: NewSlug::Derived(Slug::derive("main")),
    };
    workspaces::create(pool, new).await.unwrap().id
}

/// Makes the input of a creation by `author` in `workspace_id`.
fn new_project(author: &Actor, workspace_id: WorkspaceId) -> NewProject {
    NewProject {
        workspace_id,
        created_by: author.id,
        name: ProjectName::parse("alpha".to_owned()).unwrap(),
        description: ProjectDescription::default(),
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
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn create_rolls_back_the_hook_writes_on_a_refusal(pool: PgPool) {
    // given
    let author = actor(&pool).await;
    let hooks = hooks(Edition::builder("test").before_create(KIND, Arc::new(WriteThenReject)));

    // when
    let new = new_project(&author, workspace(&pool, &author).await);
    let result = service::create(&pool, &hooks, author, new).await;

    // then
    assert!(matches!(result, Err(Error::Rejected(_))));
    assert_eq!(actor_count(&pool).await, 1);
}

/// The hooks get the kind, the actor and the workspace, on create and on delete.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn hooks_get_the_kind_the_actor_and_the_workspace(pool: PgPool) {
    // given
    let author = actor(&pool).await;
    let owner = actor(&pool).await;
    let main = workspace(&pool, &owner).await;
    let log = Log::default();
    let hook = record("hook", &log, false);
    let hooks = hooks(
        Edition::builder("test")
            .before_create(KIND, hook.clone())
            .after_delete(KIND, hook),
    );

    // when
    let new = new_project(&author, main);
    let project = service::create(&pool, &hooks, author.clone(), new)
        .await
        .unwrap();
    service::delete(&pool, &hooks, &NoGate, author.clone(), project.id)
        .await
        .unwrap();

    // then
    let call = ("hook", KIND, author.id, main);
    assert_eq!(*log.lock().unwrap(), vec![call, call]);
}

/// The hooks run in registration order, and the first refusal stops the next ones.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
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
    let new = new_project(&author, workspace(&pool, &author).await);
    let result = service::create(&pool, &hooks, author, new).await;

    // then
    assert!(matches!(result, Err(Error::Rejected(_))));
    let names: Vec<_> = log.lock().unwrap().iter().map(|call| call.0).collect();
    assert_eq!(names, vec!["first", "second"]);
}

/// A hook of another kind does not run for a project.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn hooks_of_another_kind_do_not_run(pool: PgPool) {
    // given
    let author = actor(&pool).await;
    let log = Log::default();
    let other = ResourceKind::new("other");
    let hooks = hooks(Edition::builder("test").before_create(other, record("other", &log, true)));

    // when
    let new = new_project(&author, workspace(&pool, &author).await);
    let result = service::create(&pool, &hooks, author, new).await;

    // then
    assert!(result.is_ok());
    assert!(log.lock().unwrap().is_empty());
}
