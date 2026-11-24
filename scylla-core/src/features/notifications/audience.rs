//! The people that get a row of each kind of the core.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::invitations::{self, InvitationId};
use scylla_extension::{
    async_trait, Audience, AudienceContext, EventRow, NotificationError, Recipient,
};
use sqlx::PgPool;

use super::kinds::PERSON;
use super::store::rows;
use crate::features::tenancy::invitations as invitation_service;
use crate::Error;

/// Makes the error of the contract.
fn failed(error: Error) -> NotificationError {
    NotificationError(error.to_string())
}

/// Returns the person of a fact, or no one if its object is not a person.
fn person(event: &EventRow) -> Option<ActorId> {
    event.subject_id(PERSON)
}

/// The user with the verified email of an open invitation.
pub(super) struct Invitee;

#[async_trait]
impl Audience for Invitee {
    async fn recipients(
        &self,
        ctx: &AudienceContext<'_>,
        event: &EventRow,
    ) -> Result<Vec<Recipient>, NotificationError> {
        let Some(id) = event.subject_id::<_>(invitations::KIND.as_str()) else {
            return Ok(Vec::new());
        };
        let id: InvitationId = id;
        let user = invitation_service::invitee(ctx.pool, id).await;
        Ok(user
            .map_err(failed)?
            .map(Recipient::about)
            .into_iter()
            .collect())
    }
}

/// The person of a fact, for a reason.
pub(super) struct Person(pub(super) &'static str);

#[async_trait]
impl Audience for Person {
    async fn recipients(
        &self,
        _ctx: &AudienceContext<'_>,
        event: &EventRow,
    ) -> Result<Vec<Recipient>, NotificationError> {
        let found = person(event).map(|user| Recipient::new(user, self.0));
        Ok(found.into_iter().collect())
    }
}

/// Returns the person of a fact, and each other actor of a list with a reason.
fn with_others(event: &EventRow, others: Vec<ActorId>, reason: &'static str) -> Vec<Recipient> {
    let person = person(event);
    person
        .map(Recipient::about)
        .into_iter()
        .chain(
            others
                .into_iter()
                .filter(|other| Some(*other) != person)
                .map(|other| Recipient::new(other, reason)),
        )
        .collect()
}

/// The person of a fact, and the other owners of its workspace.
pub(super) struct PersonAndOwners;

#[async_trait]
impl Audience for PersonAndOwners {
    async fn recipients(
        &self,
        ctx: &AudienceContext<'_>,
        event: &EventRow,
    ) -> Result<Vec<Recipient>, NotificationError> {
        let owners = match event.event.workspace {
            Some(workspace) => read(ctx.pool, |tx| Box::pin(rows::owners(tx, workspace))).await,
            None => Ok(Vec::new()),
        };
        Ok(with_others(event, owners.map_err(failed)?, "owner"))
    }
}

/// The person of a fact, and the other instance admins.
pub(super) struct PersonAndAdmins;

#[async_trait]
impl Audience for PersonAndAdmins {
    async fn recipients(
        &self,
        ctx: &AudienceContext<'_>,
        event: &EventRow,
    ) -> Result<Vec<Recipient>, NotificationError> {
        let admins = read(ctx.pool, |tx| Box::pin(rows::admins(tx))).await;
        Ok(with_others(event, admins.map_err(failed)?, "admin"))
    }
}

/// A read of actors in a short transaction.
type ActorsFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<ActorId>, Error>> + Send + 'a>>;

/// Runs a read of actors in a short transaction.
async fn read<F>(pool: &PgPool, query: F) -> Result<Vec<ActorId>, Error>
where
    F: for<'a, 'c> FnOnce(&'a mut scylla_extension::Tx<'c>) -> ActorsFuture<'a>,
{
    let mut tx = pool.begin().await?;
    let actors = query(&mut tx).await?;
    tx.commit().await?;
    Ok(actors)
}
