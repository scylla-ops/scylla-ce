//! The image of the avatar of a user: the PNG of an upload, and the delete of the old image.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::images::Upload;
use scylla_extension::Tx;

use super::store;
use crate::features::platform::images;
use crate::Error;

/// Makes the PNG of a new image of an avatar before the transaction, as the decode takes time.
/// `Some(None)` removes the image.
pub(super) async fn prepare(
    change: Option<Option<Upload>>,
) -> Result<Option<Option<Vec<u8>>>, Error> {
    match change {
        Some(Some(upload)) => Ok(Some(Some(
            images::service::square_png(upload, "avatar_image").await?,
        ))),
        Some(None) => Ok(Some(None)),
        None => Ok(None),
    }
}

/// Sets the image of the avatar of a user, whose row the transaction locked: keeps the PNG of a
/// new image, then deletes the old image. `None` removes the image.
pub(super) async fn set_image(
    tx: &mut Tx<'_>,
    user: ActorId,
    png: Option<Vec<u8>>,
) -> Result<(), Error> {
    // Keep the new image, and set it.
    let old = store::avatar_image(tx, user).await?;
    let new = match png {
        Some(png) => Some(images::store::insert(tx, user, &png).await?),
        None => None,
    };
    store::set_avatar_image(tx, user, new).await?;

    // Delete the old image: no row shows it now.
    if let Some(old) = old {
        images::store::delete(tx, old).await?;
    }
    Ok(())
}
