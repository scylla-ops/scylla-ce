//! The logic of the records of the consent.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::consent::{ConsentId, NewConsentRecord};
use sqlx::PgPool;
use time::OffsetDateTime;

use super::store;
use crate::Error;

/// Records a choice of a browser, for a user or for nobody. The first choice of a browser gets
/// a new consent id. Returns the consent id and the time of the record.
pub(super) async fn record(
    pool: &PgPool,
    record: &NewConsentRecord,
    user: Option<ActorId>,
) -> Result<(ConsentId, OffsetDateTime), Error> {
    let consent = record.consent.unwrap_or_else(ConsentId::generate);
    let mut tx = pool.begin().await?;
    let at = store::insert(&mut tx, consent, record, user).await?;
    tx.commit().await?;
    Ok((consent, at))
}
