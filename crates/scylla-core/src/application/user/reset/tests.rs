//! The reset links through the engine, on stub ports.

use super::*;
use crate::application::{PrincipalAccess, UserSession};
use crate::domain::caller::CallerContext;
use crate::domain::clock;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{SessionId, UserId};
use crate::domain::permission::Permission;
use crate::domain::user::{
    DisplayName, Password, PasswordReset, RESET_LINK_INVALID, RESET_TOKEN_LEN, ResetToken, User,
};
use crate::test_support::authz::{RecordingPermissionService, actions};
use crate::test_support::stubs::{StubAccounts, StubHash, StubUsers, alice, plain_hash};
use crate::test_support::users::UserBuilder;
use async_trait::async_trait;
use scylla_extension::Actions;
use std::sync::Mutex;
use std::time::Duration as StdDuration;
use tokio::sync::Semaphore;

const PASSWORD: &str = "SecurePass123!";
const NEW_PASSWORD: &str = "NewPass123!";
const PUBLIC_URL: &str = "https://scylla.example.com/";

/// Records each message. A held sender waits for `release` before it records.
#[derive(Default)]
struct Outbox {
    sent: Mutex<Vec<PasswordResetMessage>>,
    gate: Option<Semaphore>,
    fails: bool,
}

impl Outbox {
    fn held() -> Self {
        Self {
            gate: Some(Semaphore::new(0)),
            ..Self::default()
        }
    }

    fn failing() -> Self {
        Self {
            fails: true,
            ..Self::default()
        }
    }

    fn release(&self) {
        if let Some(gate) = &self.gate {
            gate.add_permits(1);
        }
    }

    fn sent(&self) -> Vec<PasswordResetMessage> {
        self.sent.lock().unwrap().clone()
    }

    async fn wait_for(&self, count: usize) {
        tokio::time::timeout(StdDuration::from_secs(5), async {
            while self.sent.lock().unwrap().len() < count {
                tokio::time::sleep(StdDuration::from_millis(5)).await;
            }
        })
        .await
        .expect("the link was not delivered");
    }
}

#[async_trait]
impl PasswordResetSender for Outbox {
    fn delivery(&self) -> PasswordResetDelivery {
        PasswordResetDelivery::Mail
    }

    async fn send(&self, message: &PasswordResetMessage) -> DomainResult<()> {
        if let Some(gate) = &self.gate {
            gate.acquire().await.unwrap().forget();
        }
        if self.fails {
            return Err(DomainError::infrastructure("the mail server is down"));
        }
        self.sent.lock().unwrap().push(message.clone());
        Ok(())
    }
}

/// The `StubAccounts` of a lab. `issue_reset` waits for `release` when the store is held, and
/// fails when the store is broken.
struct Store {
    inner: Arc<StubAccounts>,
    gate: Option<Semaphore>,
    broken: bool,
}

impl Store {
    fn release(&self) {
        if let Some(gate) = &self.gate {
            gate.add_permits(1);
        }
    }
}

#[async_trait]
impl AccountRepository for Store {
    async fn update_signed_out(&self, user: &User, keep: Option<&SessionId>) -> DomainResult<User> {
        self.inner.update_signed_out(user, keep).await
    }
    async fn revoke_sessions(
        &self,
        user_id: &UserId,
        keep: Option<&SessionId>,
    ) -> DomainResult<u64> {
        self.inner.revoke_sessions(user_id, keep).await
    }
    async fn list_sessions(&self, user_id: &UserId) -> DomainResult<Vec<UserSession>> {
        self.inner.list_sessions(user_id).await
    }
    async fn revoke_session(&self, user_id: &UserId, id: &SessionId) -> DomainResult<bool> {
        self.inner.revoke_session(user_id, id).await
    }
    async fn issue_reset(
        &self,
        reset: &PasswordReset,
        cooldown: Option<Duration>,
    ) -> DomainResult<bool> {
        if let Some(gate) = &self.gate {
            gate.acquire().await.unwrap().forget();
        }
        if self.broken {
            return Err(DomainError::infrastructure("the database is down"));
        }
        self.inner.issue_reset(reset, cooldown).await
    }
    async fn find_reset(&self, token: &ResetToken) -> DomainResult<PasswordReset> {
        self.inner.find_reset(token).await
    }
    async fn redeem_reset(&self, reset: &PasswordReset, user: &User) -> DomainResult<User> {
        self.inner.redeem_reset(reset, user).await
    }
    async fn list_access(&self, user_id: &UserId) -> DomainResult<Vec<PrincipalAccess>> {
        self.inner.list_access(user_id).await
    }
}

struct Lab {
    actions: Actions,
    uc: PasswordResetUseCases,
    users: Arc<StubUsers>,
    accounts: Arc<StubAccounts>,
    store: Arc<Store>,
    outbox: Arc<Outbox>,
    permissions: Arc<RecordingPermissionService>,
}

impl Lab {
    async fn request(&self, email: &str) -> DomainResult<PasswordResetDelivery> {
        self.actions
            .run(
                &self.uc,
                &CallerContext::Anonymous,
                RequestPasswordReset {
                    email: Email::new(email).unwrap(),
                },
            )
            .await
    }

    async fn reset(&self, token: &ResetToken, password: &str) -> DomainResult<()> {
        self.actions
            .run(
                &self.uc,
                &CallerContext::Anonymous,
                ResetPassword {
                    token: token.clone(),
                    new_password: Password::new(password).unwrap(),
                },
            )
            .await
    }

    async fn send(&self, user: &User) -> DomainResult<PasswordResetDelivery> {
        self.actions
            .run(
                &self.uc,
                &alice(),
                SendPasswordReset {
                    id: user.id().clone(),
                },
            )
            .await
    }

    fn stored(&self, id: &UserId) -> User {
        self.users.rows()[id].clone()
    }
}

fn token_of(message: &PasswordResetMessage) -> ResetToken {
    let token = message.link.rsplit_once("#token=").unwrap().1;
    ResetToken::new(token).unwrap()
}

fn lab(outbox: Outbox) -> Lab {
    lab_with(outbox, None, false)
}

fn lab_with(outbox: Outbox, gate: Option<Semaphore>, broken: bool) -> Lab {
    let users = Arc::new(StubUsers::default());
    let accounts = Arc::new(StubAccounts::new(users.clone()));
    let store = Arc::new(Store {
        inner: accounts.clone(),
        gate,
        broken,
    });
    let outbox = Arc::new(outbox);
    let permissions = Arc::new(RecordingPermissionService::new());
    Lab {
        actions: actions(permissions.clone()),
        uc: PasswordResetUseCases::new(
            users.clone(),
            store.clone(),
            Arc::new(StubHash::plain()),
            outbox.clone(),
            ResetLinks::new(Some(PUBLIC_URL)),
        ),
        users,
        accounts,
        store,
        outbox,
        permissions,
    }
}

fn account(lab: &Lab, id: &str, active: bool) -> User {
    let user = UserBuilder::new(id)
        .id(UserId::new(id))
        .email(format!("{id}@example.com"))
        .display_name("Kevin K.")
        .password_hash(plain_hash(PASSWORD).as_str())
        .is_active(active)
        .build();
    lab.users.insert(user.clone());
    user
}

fn aged(reset: &PasswordReset, age: Duration) -> PasswordReset {
    PasswordReset::from_persistence(
        reset.id().clone(),
        reset.token().clone(),
        reset.user_id().clone(),
        reset.created_at() - age,
        reset.expires_at() - age,
        reset.used_at(),
    )
}

fn invalid(result: DomainResult<()>) -> bool {
    matches!(result, Err(DomainError::BusinessRule(m)) if m == RESET_LINK_INVALID)
}

#[tokio::test]
async fn a_request_for_an_active_account_stores_a_link_and_delivers_it() {
    let lab = lab(Outbox::default());
    let kevin = account(&lab, "kevin", true);

    let delivery = lab.request("Kevin@Example.com").await.unwrap();
    lab.outbox.wait_for(1).await;

    assert_eq!(delivery, PasswordResetDelivery::Mail);
    let message = &lab.outbox.sent()[0];
    let stored = lab.accounts.resets(kevin.id());
    assert_eq!(stored.len(), 1);
    assert_eq!(message.user_id, *kevin.id());
    assert_eq!(message.email.as_str(), "kevin@example.com");
    assert_eq!(message.username, *kevin.username());
    assert_eq!(
        message.display_name.as_ref().map(DisplayName::as_str),
        Some("Kevin K.")
    );
    assert_eq!(
        message.link,
        format!(
            "https://scylla.example.com/reset-password#token={}",
            stored[0].token().as_str()
        )
    );
    assert_eq!(message.expires_at, stored[0].expires_at());
    assert_eq!(
        stored[0].expires_at() - stored[0].created_at(),
        Duration::hours(1)
    );
    assert!(lab.permissions.permissions().is_empty());
}

#[tokio::test]
async fn the_answer_is_the_same_for_an_unknown_email_and_an_inactive_account() {
    let lab = lab(Outbox::default());
    let idle = account(&lab, "idle", false);
    account(&lab, "kevin", true);

    let unknown = lab.request("ghost@example.com").await.unwrap();
    let inactive = lab.request("idle@example.com").await.unwrap();
    let known = lab.request("kevin@example.com").await.unwrap();
    lab.outbox.wait_for(1).await;

    assert_eq!(unknown, known);
    assert_eq!(inactive, known);
    assert!(lab.accounts.resets(idle.id()).is_empty());
    assert_eq!(lab.outbox.sent().len(), 1);
    assert_eq!(lab.outbox.sent()[0].user_id, UserId::new("kevin"));
}

#[tokio::test]
async fn the_answer_comes_before_the_delivery() {
    let lab = lab(Outbox::held());
    account(&lab, "kevin", true);

    lab.request("kevin@example.com").await.unwrap();

    assert!(lab.outbox.sent().is_empty());
    lab.outbox.release();
    lab.outbox.wait_for(1).await;
}

#[tokio::test]
async fn the_answer_does_not_wait_for_the_store() {
    let lab = lab_with(Outbox::default(), Some(Semaphore::new(0)), false);
    let kevin = account(&lab, "kevin", true);

    let known = lab.request("kevin@example.com").await.unwrap();
    let unknown = lab.request("ghost@example.com").await.unwrap();

    assert_eq!(known, unknown);
    assert!(lab.accounts.resets(kevin.id()).is_empty());
    assert!(lab.outbox.sent().is_empty());
    lab.store.release();
    lab.outbox.wait_for(1).await;
    assert_eq!(lab.accounts.resets(kevin.id()).len(), 1);
}

#[tokio::test]
async fn a_store_error_after_the_answer_reaches_no_caller() {
    let lab = lab_with(Outbox::default(), None, true);
    account(&lab, "kevin", true);

    let known = lab.request("kevin@example.com").await.unwrap();
    let unknown = lab.request("ghost@example.com").await.unwrap();

    assert_eq!(known, unknown);
    assert!(lab.outbox.sent().is_empty());
}

#[tokio::test]
async fn a_second_link_within_a_minute_is_not_stored_and_not_delivered() {
    let lab = lab(Outbox::default());
    let kevin = account(&lab, "kevin", true);
    let email = kevin.email().unwrap();
    let first = lab.uc.new_reset(&kevin, email).unwrap();
    let second = lab.uc.new_reset(&kevin, email).unwrap();

    let stored = issue_and_deliver(&*lab.store, &*lab.outbox, first.clone())
        .await
        .unwrap();
    let again = issue_and_deliver(&*lab.store, &*lab.outbox, second)
        .await
        .unwrap();

    assert!(stored);
    assert!(!again);
    assert_eq!(lab.outbox.sent().len(), 1);
    let kept = lab.accounts.resets(kevin.id());
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].id(), first.reset.id());
}

#[tokio::test]
async fn a_send_that_fails_in_the_task_keeps_the_link() {
    let lab = lab(Outbox::failing());
    let kevin = account(&lab, "kevin", true);
    let new = lab.uc.new_reset(&kevin, kevin.email().unwrap()).unwrap();

    let err = issue_and_deliver(&*lab.store, &*lab.outbox, new)
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Infrastructure(_)), "{err}");
    assert_eq!(lab.accounts.resets(kevin.id()).len(), 1);
}

#[tokio::test]
async fn a_request_after_the_minute_cancels_the_earlier_link() {
    let lab = lab(Outbox::default());
    let kevin = account(&lab, "kevin", true);
    let old = aged(
        &PasswordReset::create(
            kevin.id().clone(),
            mint_reset_token().unwrap(),
            RESET_LINK_TTL,
        ),
        Duration::seconds(61),
    );
    lab.accounts.put_reset(old.clone());

    lab.request("kevin@example.com").await.unwrap();
    lab.outbox.wait_for(1).await;

    let resets = lab.accounts.resets(kevin.id());
    assert_eq!(resets.len(), 1);
    assert_ne!(resets[0].id(), old.id());
    assert!(invalid(lab.reset(old.token(), NEW_PASSWORD).await));
}

#[tokio::test]
async fn a_reset_sets_the_password_once_and_signs_out_everywhere() {
    let lab = lab(Outbox::default());
    let kevin = account(&lab, "kevin", true);
    lab.accounts.open_session(kevin.id());
    lab.accounts.open_session(kevin.id());
    lab.request("kevin@example.com").await.unwrap();
    lab.outbox.wait_for(1).await;
    let token = token_of(&lab.outbox.sent()[0]);

    lab.reset(&token, NEW_PASSWORD).await.unwrap();

    assert_eq!(
        lab.stored(kevin.id()).password_hash(),
        &plain_hash(NEW_PASSWORD)
    );
    assert!(lab.accounts.sessions(kevin.id()).is_empty());
    let resets = lab.accounts.resets(kevin.id());
    assert_eq!(resets.len(), 1);
    assert!(resets[0].used_at().is_some());

    assert!(invalid(lab.reset(&token, "OtherPass123!").await));
    assert_eq!(
        lab.stored(kevin.id()).password_hash(),
        &plain_hash(NEW_PASSWORD)
    );
}

#[tokio::test]
async fn an_unknown_expired_or_inactive_link_gives_one_error() {
    let lab = lab(Outbox::default());
    let kevin = account(&lab, "kevin", true);
    let idle = account(&lab, "idle", false);
    let expired = aged(
        &PasswordReset::create(
            kevin.id().clone(),
            mint_reset_token().unwrap(),
            RESET_LINK_TTL,
        ),
        Duration::minutes(61),
    );
    let of_idle = PasswordReset::create(
        idle.id().clone(),
        mint_reset_token().unwrap(),
        RESET_LINK_TTL,
    );
    lab.accounts.put_reset(expired.clone());
    lab.accounts.put_reset(of_idle.clone());

    for token in [
        mint_reset_token().unwrap(),
        expired.token().clone(),
        of_idle.token().clone(),
    ] {
        assert!(invalid(lab.reset(&token, NEW_PASSWORD).await));
    }
    assert_eq!(
        lab.stored(kevin.id()).password_hash(),
        &plain_hash(PASSWORD)
    );
    assert_eq!(lab.stored(idle.id()).password_hash(), &plain_hash(PASSWORD));
}

#[tokio::test]
async fn an_administrator_sends_a_link_inline_without_the_minute_rule() {
    let lab = lab(Outbox::default());
    let kevin = account(&lab, "kevin", true);

    let first = lab.send(&kevin).await.unwrap();
    assert_eq!(lab.outbox.sent().len(), 1);
    lab.send(&kevin).await.unwrap();

    assert_eq!(first, PasswordResetDelivery::Mail);
    assert_eq!(lab.outbox.sent().len(), 2);
    let resets = lab.accounts.resets(kevin.id());
    assert_eq!(resets.len(), 1);
    assert_eq!(token_of(&lab.outbox.sent()[1]), *resets[0].token());
    assert_eq!(
        lab.permissions.permissions(),
        vec![
            Permission::UpdateUser(kevin.id().clone()),
            Permission::UpdateUser(kevin.id().clone())
        ]
    );
}

#[tokio::test]
async fn a_link_for_an_inactive_account_or_an_account_without_an_email_is_refused() {
    let lab = lab(Outbox::default());
    let idle = account(&lab, "idle", false);
    let legacy = UserBuilder::new("legacy").id(UserId::new("legacy")).build();
    lab.users.insert(legacy.clone());

    for user in [&idle, &legacy] {
        let err = lab.send(user).await.unwrap_err();
        assert!(matches!(err, DomainError::BusinessRule(_)), "{err}");
    }
    assert!(lab.outbox.sent().is_empty());
}

#[tokio::test]
async fn a_failed_send_is_the_error_of_the_administrator() {
    let lab = lab(Outbox::failing());
    let kevin = account(&lab, "kevin", true);

    let err = lab.send(&kevin).await.unwrap_err();

    assert!(matches!(err, DomainError::Infrastructure(_)), "{err}");
}

#[test]
fn a_minted_token_is_43_characters_of_base64url_and_never_repeats() {
    let a = mint_reset_token().unwrap();
    let b = mint_reset_token().unwrap();
    assert_eq!(a.as_str().len(), RESET_TOKEN_LEN);
    assert_ne!(a, b);
}

#[test]
fn a_link_starts_with_the_public_url_or_is_relative() {
    let token = ResetToken::new("abcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123-_Z").unwrap();
    assert_eq!(
        ResetLinks::new(Some("http://127.0.0.1:8080/")).link(&token),
        "http://127.0.0.1:8080/reset-password#token=abcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123-_Z"
    );
    assert_eq!(
        ResetLinks::new(None).link(&token),
        "/reset-password#token=abcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123-_Z"
    );
}

#[test]
fn a_message_never_prints_its_link() {
    let message = PasswordResetMessage {
        user_id: UserId::new("kevin"),
        email: Email::new("kevin@example.com").unwrap(),
        username: crate::domain::user::Username::new("kevin").unwrap(),
        display_name: None,
        link: "https://x/reset-password#token=secret".into(),
        expires_at: clock::now(),
    };
    let printed = format!("{message:?}");
    assert!(!printed.contains("secret"), "{printed}");
    assert!(!printed.contains("kevin@example.com"), "{printed}");
}
