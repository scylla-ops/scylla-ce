//! Unit tests of the library service: the hooks and the door of an edition, which the API cannot
//! show.

use std::sync::{Arc, Mutex};

use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::users::{Email, NewUser};
use scylla_domain::projects::pipelines::{Node, NodeId, Runtime, Step};
use scylla_domain::tenancy::library::{NewFragment, NewOrigin, NewVersion, KIND, VERSION_KIND};
use scylla_domain::tenancy::slugs::Slug;
use scylla_domain::tenancy::workspaces::{NewSlug, NewWorkspace, WorkspaceId};
use scylla_domain::{Name, ResourceKind};
use scylla_extension::{
    async_trait, AfterDelete, BeforeCreate, BeforeDelete, CreateContext, DeleteContext, Edition,
    HookError, Hooks, Rejection, Tx,
};
use sqlx::PgPool;

use super::{origin_service, service};
use crate::features::identity::accounts;
use crate::features::tenancy::workspaces;
use crate::Error;

/// The log of the `Record` hooks: the event, the kind and the object of each call.
type Log = Arc<Mutex<Vec<(&'static str, ResourceKind, Option<String>)>>>;

/// A hook that records each call, and refuses if `reject` is set.
struct Record {
    /// The shared log.
    log: Log,
    /// Refuse each call after the record.
    reject: bool,
}

impl Record {
    /// Adds the call to the log, then accepts or refuses it.
    fn record(
        &self,
        event: &'static str,
        kind: ResourceKind,
        object: &Option<String>,
    ) -> Result<(), HookError> {
        self.log.lock().unwrap().push((event, kind, object.clone()));
        if self.reject {
            return Err(Rejection::new("TEST_REJECTED", "rejected by the test hook").into());
        }
        Ok(())
    }
}

#[async_trait]
impl BeforeCreate for Record {
    async fn before_create(&self, _tx: &mut Tx<'_>, ctx: &CreateContext) -> Result<(), HookError> {
        self.record("before_create", ctx.kind, &ctx.object)
    }
}

#[async_trait]
impl BeforeDelete for Record {
    async fn before_delete(&self, _tx: &mut Tx<'_>, ctx: &DeleteContext) -> Result<(), HookError> {
        self.record("before_delete", ctx.kind, &ctx.object)
    }
}

#[async_trait]
impl AfterDelete for Record {
    async fn after_delete(&self, _tx: &mut Tx<'_>, ctx: &DeleteContext) -> Result<(), HookError> {
        self.record("after_delete", ctx.kind, &ctx.object)
    }
}

/// Makes a `Record` hook on `log`.
fn record(log: &Log, reject: bool) -> Arc<Record> {
    Arc::new(Record {
        log: Arc::clone(log),
        reject,
    })
}

/// Creates a user with its personal workspace, and a workspace that it owns.
async fn owner(pool: &PgPool) -> (Actor, WorkspaceId) {
    let new = NewUser {
        email: Email::parse(format!("{}@test.example", ActorId::generate())).unwrap(),
        email_verified: true,
        display_name: Name::parse("Test".to_owned()).unwrap(),
        handle: None,
        password: None,
    };
    let actor = accounts::create_user(pool, new).await.unwrap();
    let workspace = NewWorkspace {
        creator: actor.id,
        name: Name::parse("main".to_owned()).unwrap(),
        slug: NewSlug::Derived(Slug::derive("main")),
    };
    let workspace = workspaces::create(pool, workspace).await.unwrap().id;
    (actor, workspace)
}

/// Makes the content of a version with one node.
fn version() -> NewVersion {
    let node = Node {
        id: NodeId::parse("build".to_owned()).unwrap(),
        needs: Vec::new(),
        step: Step::Exec(vec!["make".to_owned()]),
        env: Vec::new(),
        timeout_seconds: Node::DEFAULT_TIMEOUT,
        retries: 0,
        working_dir: None,
        runtime: Runtime::Process,
    };
    NewVersion {
        nodes: vec![node],
        parameters: Vec::new(),
        changes: Vec::new(),
    }
}

/// Makes the input of a fragment with a first number.
fn new_fragment(actor: &Actor, workspace_id: WorkspaceId, origin: Option<u32>) -> NewFragment {
    NewFragment {
        workspace_id,
        created_by: actor.id,
        name: NodeId::parse("build".to_owned()).unwrap(),
        description: String::new(),
        tags: Vec::new(),
        origin: origin.map(|version| NewOrigin {
            registry: "https://registry.test".to_owned(),
            package_id: "acme.build".to_owned(),
            package_name: "@acme/build".to_owned(),
            version,
            linked: true,
        }),
        version: version(),
    }
}

/// Returns the hooks of a test edition.
fn hooks(edition: scylla_extension::EditionBuilder) -> Arc<Hooks> {
    edition.build().hooks()
}

/// The hook of a new version gets the kind `fragment_version` and the fragment as its object,
/// and its refusal adds no version.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn publish_version_names_the_fragment_to_the_hooks(pool: PgPool) {
    // given
    let (actor, workspace) = owner(&pool).await;
    let log = Log::default();
    let fragment = service::create(
        &pool,
        &Hooks::default(),
        actor.clone(),
        new_fragment(&actor, workspace, None),
    )
    .await
    .unwrap();
    let refusing = hooks(Edition::builder("test").before_create(VERSION_KIND, record(&log, true)));

    // when
    let result =
        service::publish_version(&pool, &refusing, actor.clone(), fragment.id, version()).await;

    // then
    assert!(matches!(result, Err(Error::Rejected(_))));
    let object = Some(format!("fragment:{}", fragment.id));
    assert_eq!(
        *log.lock().unwrap(),
        vec![("before_create", VERSION_KIND, object)]
    );
    let read = service::get(&pool, fragment.id, actor.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.versions.len(), 1);
}

/// A refusal of a `BeforeDelete` hook keeps the fragment. With no refusal, the hooks run before
/// and after the delete, with the fragment as their object.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn delete_runs_the_hooks_around_the_delete(pool: PgPool) {
    // given
    let (actor, workspace) = owner(&pool).await;
    let log = Log::default();
    let fragment = service::create(
        &pool,
        &Hooks::default(),
        actor.clone(),
        new_fragment(&actor, workspace, None),
    )
    .await
    .unwrap();
    let refusing = hooks(Edition::builder("test").before_delete(KIND, record(&log, true)));
    let accepting = hooks(
        Edition::builder("test")
            .before_delete(KIND, record(&log, false))
            .after_delete(KIND, record(&log, false)),
    );

    // when
    let refused = service::delete(&pool, &refusing, actor.clone(), fragment.id).await;
    let kept = service::find(&pool, fragment.id).await.unwrap();
    service::delete(&pool, &accepting, actor.clone(), fragment.id)
        .await
        .unwrap();

    // then
    assert!(matches!(refused, Err(Error::Rejected(_))));
    assert!(kept.is_some());
    assert!(service::find(&pool, fragment.id).await.unwrap().is_none());
    let object = Some(format!("fragment:{}", fragment.id));
    let calls: Vec<_> = log.lock().unwrap().iter().map(|call| call.0).collect();
    assert_eq!(calls, ["before_delete", "before_delete", "after_delete"]);
    assert!(log
        .lock()
        .unwrap()
        .iter()
        .all(|call| call.1 == KIND && call.2 == object));
}

/// A linked fragment keeps the numbers of its package, with gaps, refuses a number that is not
/// newer, and refuses a version made by hand.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn a_linked_fragment_keeps_the_numbers_of_its_package(pool: PgPool) {
    // given
    let (actor, workspace) = owner(&pool).await;
    let hooks = Hooks::default();
    let new = new_fragment(&actor, workspace, Some(3));
    let fragment = service::create(&pool, &hooks, actor.clone(), new)
        .await
        .unwrap();

    // when
    origin_service::accept(&pool, &hooks, actor.clone(), fragment.id, 5, version())
        .await
        .unwrap();
    let older =
        origin_service::accept(&pool, &hooks, actor.clone(), fragment.id, 4, version()).await;
    let by_hand =
        service::publish_version(&pool, &hooks, actor.clone(), fragment.id, version()).await;
    let mut tx = pool.begin().await.unwrap();
    let numbers: Vec<_> = service::versions(&mut tx, fragment.id)
        .await
        .unwrap()
        .iter()
        .map(|v| v.number)
        .collect();

    // then
    assert!(matches!(older, Err(Error::Rejected(r)) if r.code == "FRAGMENT_VERSION_NOT_NEWER"));
    assert!(matches!(by_hand, Err(Error::Rejected(r)) if r.code == "FRAGMENT_LINKED"));
    assert_eq!(numbers, [3, 5]);
}

/// A detached fragment is a fork from its latest version: it takes versions made by hand, and
/// its origin keeps the version that it dismissed.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn a_detached_fragment_is_a_fork(pool: PgPool) {
    // given
    let (actor, workspace) = owner(&pool).await;
    let hooks = Hooks::default();
    let new = new_fragment(&actor, workspace, Some(2));
    let fragment = service::create(&pool, &hooks, actor.clone(), new)
        .await
        .unwrap();

    // when
    let skipped = origin_service::skip(&pool, actor.clone(), fragment.id, 3)
        .await
        .unwrap();
    let detached = origin_service::detach(&pool, actor.clone(), fragment.id)
        .await
        .unwrap();
    let dismissed = origin_service::dismiss(&pool, actor.clone(), fragment.id, 4)
        .await
        .unwrap();
    let not_linked = origin_service::skip(&pool, actor.clone(), fragment.id, 5).await;
    let changed = service::publish_version(&pool, &hooks, actor, fragment.id, version()).await;

    // then
    assert_eq!(skipped.origin.unwrap().skipped, [3]);
    let origin = detached.origin.unwrap();
    assert!(!origin.linked && origin.skipped.is_empty());
    assert_eq!(origin.base_version, 2);
    assert_eq!(dismissed.origin.unwrap().dismissed, Some(4));
    assert!(matches!(not_linked, Err(Error::Rejected(r)) if r.code == "FRAGMENT_NOT_LINKED"));
    assert_eq!(changed.unwrap().versions.len(), 2);
}
