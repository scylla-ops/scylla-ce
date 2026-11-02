use std::sync::Mutex;

use scylla_domain::projects::{ListProjects, NewProject, ProjectName, KIND};
use scylla_domain::users::UserId;
use scylla_domain::ResourceKind;
use scylla_extension::{
    async_trait, AllowAll, CreateContext, DeleteContext, GuardError, Rejection, ResourceGuard,
};
use sqlx::PgPool;
use uuid::Uuid;

use super::service;
use crate::db::Tx;
use crate::features::users;
use crate::Error;

struct WriteThenReject;

#[async_trait]
impl ResourceGuard for WriteThenReject {
    async fn before_create(&self, tx: &mut Tx<'_>, _ctx: &CreateContext) -> Result<(), GuardError> {
        sqlx::query!(
            "INSERT INTO users (id, name) VALUES ($1, 'written by the guard')",
            Uuid::now_v7(),
        )
        .execute(&mut **tx)
        .await?;
        Err(Rejection::new("TEST_REJECTED", "rejected by the test guard").into())
    }
}

#[derive(Default)]
struct RecordDeletes(Mutex<Vec<(ResourceKind, UserId)>>);

#[async_trait]
impl ResourceGuard for RecordDeletes {
    async fn before_create(
        &self,
        _tx: &mut Tx<'_>,
        _ctx: &CreateContext,
    ) -> Result<(), GuardError> {
        Ok(())
    }

    async fn after_delete(&self, _tx: &mut Tx<'_>, ctx: &DeleteContext) -> Result<(), GuardError> {
        self.0.lock().unwrap().push((ctx.kind, ctx.owner_id));
        Ok(())
    }
}

async fn owner(pool: &PgPool) -> UserId {
    let mut tx = pool.begin().await.unwrap();
    let id = users::insert(&mut tx, "owner").await.unwrap();
    tx.commit().await.unwrap();
    id
}

fn new_project(owner_id: UserId, name: &str) -> NewProject {
    NewProject {
        owner_id,
        name: ProjectName::parse(name.to_owned()).unwrap(),
    }
}

#[sqlx::test(migrator = "crate::db::MIGRATOR")]
async fn a_rejection_rolls_back_the_guard_writes(pool: PgPool) {
    let owner = owner(&pool).await;

    let error = service::create(&pool, &WriteThenReject, new_project(owner, "alpha"))
        .await
        .unwrap_err();

    assert!(matches!(error, Error::Rejected(ref rejection) if rejection.code == "TEST_REJECTED"));
    let users = sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM users"#)
        .fetch_one(&pool)
        .await
        .unwrap();
    let projects = sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM projects"#)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((users, projects), (1, 0));
}

#[sqlx::test(migrator = "crate::db::MIGRATOR")]
async fn delete_gives_the_kind_and_the_owner_to_the_guard(pool: PgPool) {
    let owner = owner(&pool).await;
    let project = service::create(&pool, &AllowAll, new_project(owner, "alpha"))
        .await
        .unwrap();
    let guard = RecordDeletes::default();

    service::delete(&pool, &guard, project.id).await.unwrap();

    assert_eq!(*guard.0.lock().unwrap(), vec![(KIND, owner)]);
}

#[sqlx::test(migrator = "crate::db::MIGRATOR")]
async fn list_returns_pages_in_creation_order(pool: PgPool) {
    let owner = owner(&pool).await;
    let mut created = Vec::new();
    for i in 0..5 {
        let project = service::create(&pool, &AllowAll, new_project(owner, &format!("p{i}")))
            .await
            .unwrap();
        created.push(project.id);
    }

    let (mut listed, mut sizes, mut after) = (Vec::new(), Vec::new(), None);
    loop {
        let query = ListProjects {
            owner_id: owner,
            after,
            limit: 2,
        };
        let page = service::list(&pool, query).await.unwrap();
        sizes.push(page.projects.len());
        listed.extend(page.projects.iter().map(|project| project.id));
        match page.next_after {
            Some(next) => after = Some(next),
            None => break,
        }
    }

    assert_eq!(sizes, vec![2, 2, 1]);
    assert_eq!(listed, created);
}
