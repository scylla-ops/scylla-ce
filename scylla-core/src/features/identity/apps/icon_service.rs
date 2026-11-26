//! The logic of the custom icon that the admins give to an App: one transaction for each call.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::apps::{App, AppId};
use scylla_domain::platform::images::IconChange;
use sqlx::PgPool;

use super::store::{apps, icons};
use crate::features::platform::images;
use crate::Error;

/// Changes the icon of an App, and returns it. A new image replaces the old image of the App,
/// which goes. An unknown App gives `NotFound("app")`.
pub(crate) async fn set_icon(
    pool: &PgPool,
    actor: ActorId,
    id: AppId,
    change: IconChange,
) -> Result<App, Error> {
    // Make the PNG of a new image before the transaction, as the decode takes time.
    let icon = images::service::new_icon(change, "icon.image").await?;

    // Lock the App, keep a new image, and set the icon.
    let mut tx = pool.begin().await?;
    let old = icons::lock(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("app"))?;
    let icon = images::service::keep_icon(&mut tx, actor, icon).await?;
    icons::set(&mut tx, id, icon.as_ref()).await?;

    // Delete the old image, and read the App after the change.
    if let Some(old) = old {
        images::store::delete(&mut tx, old).await?;
    }
    let app = apps::find(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("app"))?;
    tx.commit().await?;
    Ok(app)
}
