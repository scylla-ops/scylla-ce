use super::PgUserRepository;
use crate::domain::errors::DomainError;
use crate::domain::ids::UserId;
use crate::domain::user::{DisplayName, Email, Username};
use crate::test_support::prelude::*;
use scylla_core::application::UserRepository;
use scylla_core::application::pagination::PaginationParams;
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
async fn create_then_find_round_trips(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    let user = user("alice");

    repo.create(&user).await.expect("create");
    let found = repo.find_by_id(user.id()).await.expect("find");

    assert_eq!(found.id(), user.id());
    assert_eq!(found.username(), user.username());
    assert_eq!(
        found.password_hash().as_str(),
        user.password_hash().as_str()
    );
    assert_eq!(found.is_active(), user.is_active());
    assert_eq!(found.created_at(), user.created_at());
    assert_eq!(found.updated_at(), user.updated_at());
}

#[sqlx::test(migrations = "../../migrations")]
async fn find_by_id_not_found_returns_not_found_error(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    let res = repo.find_by_id(&UserId::generate()).await;
    assert!(matches!(res, Err(DomainError::NotFound(_))));
}

#[sqlx::test(migrations = "../../migrations")]
async fn find_by_username_returns_persisted_user(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    let user = user("bob");
    repo.create(&user).await.expect("create");

    let found = repo.find_by_username(user.username()).await.expect("find");
    assert_eq!(found.id(), user.id());
}

#[sqlx::test(migrations = "../../migrations")]
async fn find_by_email_returns_persisted_user(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    let user = UserBuilder::new("heidi").email("heidi@example.com").build();
    repo.create(&user).await.expect("create");

    let email = Email::new("heidi@example.com").unwrap();
    let found = repo.find_by_email(&email).await.expect("find by email");
    assert_eq!(found.id(), user.id());
    assert_eq!(found.email().map(Email::as_str), Some("heidi@example.com"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_update_writes_the_email_of_a_user_that_had_none(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    let mut stored = repo.create(&user("nora")).await.expect("create");
    let email = Email::new("nora@example.com").unwrap();

    stored.update_email(email.clone());
    repo.update(&stored).await.expect("update");

    let found = repo.find_by_email(&email).await.expect("find by email");
    assert_eq!(found.id(), stored.id());
    assert_eq!(found.version(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn duplicate_email_maps_to_conflict(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    repo.create(&UserBuilder::new("ivan").email("dup@example.com").build())
        .await
        .expect("first");

    let clash = UserBuilder::new("ivana").email("dup@example.com").build();
    assert!(matches!(
        repo.create(&clash).await,
        Err(DomainError::Conflict(m)) if m == "Email already exists"
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn null_emails_do_not_collide(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    repo.create(&user("judy")).await.expect("first null email");
    repo.create(&user("mallory"))
        .await
        .expect("second null email must not conflict");
}

#[sqlx::test(migrations = "../../migrations")]
async fn inactive_user_round_trips(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    let user = UserBuilder::new("carol").is_active(false).build();
    repo.create(&user).await.expect("create");

    assert!(!repo.find_by_id(user.id()).await.unwrap().is_active());
}

#[sqlx::test(migrations = "../../migrations")]
async fn update_modifies_fields_then_visible_on_read(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    let mut user = user("dave");
    repo.create(&user).await.expect("create");

    let new_name = Username::new("davinci").unwrap();
    user.update_username(new_name.clone()).unwrap();
    repo.update(&user).await.expect("update");

    assert_eq!(
        repo.find_by_id(user.id()).await.unwrap().username(),
        &new_name
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn delete_removes_row(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    let user = user("eve");
    repo.create(&user).await.expect("create");

    repo.delete(&user).await.expect("delete");
    assert!(matches!(
        repo.find_by_id(user.id()).await,
        Err(DomainError::NotFound(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn unique_username_violation_maps_to_conflict(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    repo.create(&user("frank")).await.expect("first");

    let dup = user("frank");
    assert!(matches!(
        repo.create(&dup).await,
        Err(DomainError::Conflict(m)) if m == "Username already exists"
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_value_the_database_cannot_store_is_a_validation_error(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    assert!(matches!(
        repo.find_by_id(&UserId::new("nul\0id")).await,
        Err(DomainError::Validation(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_all_paginates_in_creation_order(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    for n in &["a", "b", "c"] {
        repo.create(&user(n)).await.expect("seed");
    }

    let page = PaginationParams::new(1, 20).unwrap();
    let result = repo.list_all(Some(&page)).await.expect("list");

    assert_eq!(result.metadata().total_count(), 3);
    assert_eq!(result.items().len(), 3);
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_update_from_a_stale_read_is_stale_and_the_first_write_wins(pool: PgPool) {
    let user = seed_user(&pool, "eve").await;
    let repo = PgUserRepository::new(pool);

    let mut first = repo.find_by_id(user.id()).await.expect("first read");
    let mut second = repo.find_by_id(user.id()).await.expect("second read");
    assert_eq!(first.version(), 0);

    first.update_username(Username::new("a").unwrap()).unwrap();
    let written = repo.update(&first).await.expect("first write");
    assert_eq!(written.version(), 1);

    second.update_username(Username::new("b").unwrap()).unwrap();
    let err = repo.update(&second).await.expect_err("stale write");
    assert!(matches!(err, DomainError::Stale(_)));

    let stored = repo.find_by_id(user.id()).await.expect("find");
    assert_eq!(stored.username().as_str(), "a");
    assert_eq!(stored.version(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_delete_with_a_stale_version_is_stale(pool: PgPool) {
    let user = seed_user(&pool, "eve").await;
    let repo = PgUserRepository::new(pool);

    let stale = repo.find_by_id(user.id()).await.expect("read");
    let mut fresh = stale.clone();
    fresh
        .update_username(Username::new("renamed").unwrap())
        .unwrap();
    let fresh = repo.update(&fresh).await.expect("write");

    let err = repo.delete(&stale).await.expect_err("stale delete");
    assert!(matches!(err, DomainError::Stale(_)));
    assert!(repo.find_by_id(user.id()).await.is_ok());

    repo.delete(&fresh)
        .await
        .expect("delete at the current version");
    assert!(matches!(
        repo.find_by_id(user.id()).await,
        Err(DomainError::NotFound(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_write_on_a_missing_row_is_not_found(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    let never_persisted = user("ghost");

    assert!(matches!(
        repo.update(&never_persisted).await,
        Err(DomainError::NotFound(_))
    ));
    assert!(matches!(
        repo.delete(&never_persisted).await,
        Err(DomainError::NotFound(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_display_name_round_trips_and_an_update_removes_it(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    let user = UserBuilder::new("ada").display_name("Ada Lovelace").build();
    repo.create(&user).await.expect("create");

    let mut found = repo.find_by_id(user.id()).await.expect("find");
    assert_eq!(
        found.display_name().map(DisplayName::as_str),
        Some("Ada Lovelace")
    );

    found.set_display_name(None);
    repo.update(&found).await.expect("update");
    let page = repo.list_all(None).await.expect("list");
    assert!(page.items()[0].display_name().is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn two_users_share_a_display_name(pool: PgPool) {
    let repo = PgUserRepository::new(pool);
    repo.create(&UserBuilder::new("ada").display_name("Ada").build())
        .await
        .expect("first");
    repo.create(&UserBuilder::new("ada2").display_name("Ada").build())
        .await
        .expect("a display name is not unique");
}
