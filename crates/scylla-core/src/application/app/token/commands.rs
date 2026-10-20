//! The app token exchange's writes. One block per command, in the order it runs: the struct,
//! its access, its payload types, what `Prepare` builds, what `Persist` writes.

use super::AppTokenUseCases;
use crate::domain::app::{AppSecret, AppToken};
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::AppId;
use async_trait::async_trait;
use chrono::Duration;
use scylla_extension::{
    Access, Authorized, Command, Committed, Describe, Draft, Persist, Prepare, Prepared, Run,
};
use uuid::Uuid;

const DEFAULT_APP_TOKEN_DURATION_DAYS: i64 = 30;

#[derive(Debug)]
pub struct IssueAppToken {
    pub app_id: AppId,
    pub secret: AppSecret,
}

impl Describe for IssueAppToken {
    fn access(&self) -> Access {
        Access::Public
    }
}

impl Command for IssueAppToken {
    type Staged = Draft<AppToken>;
    type Committed = AppToken;
}

/// Unknown app, inactive app, disabled secret and wrong secret all return the same opaque error.
#[async_trait]
impl Run<Prepare<IssueAppToken>> for AppTokenUseCases {
    async fn run(&self, input: Authorized<IssueAppToken>) -> DomainResult<Prepared<IssueAppToken>> {
        let cmd = input.command();
        let invalid = || DomainError::unauthorized("Invalid app credentials");
        let app = self
            .app_repo
            .find_by_id(&cmd.app_id)
            .await
            .map_err(|_| invalid())?;
        if !app.is_active() {
            return Err(invalid());
        }

        // Verify every candidate, no early break, so timing does not leak which secret matched.
        let credentials = self
            .credential_repo
            .list_enabled_by_app(&cmd.app_id)
            .await
            .map_err(|_| invalid())?;
        let mut matched: Option<&_> = None;
        for credential in &credentials {
            if self
                .hash_service
                .verify_secret(&cmd.secret, credential.secret_hash())
                .await?
            {
                matched = Some(credential);
            }
        }
        let matched = matched.ok_or_else(invalid)?;

        let token = AppToken::create(
            cmd.app_id.clone(),
            matched.id().clone(),
            Uuid::new_v4().to_string(),
            Duration::days(DEFAULT_APP_TOKEN_DURATION_DAYS),
        );
        Ok(input.prepared(Draft::new(token)))
    }
}

#[async_trait]
impl Run<Persist<IssueAppToken>> for AppTokenUseCases {
    async fn run(&self, input: Prepared<IssueAppToken>) -> DomainResult<Committed<IssueAppToken>> {
        input
            .commit(async |draft| {
                let token = draft.into_inner();
                self.token_repo.create(&token).await?;
                Ok(token)
            })
            .await
    }
}
