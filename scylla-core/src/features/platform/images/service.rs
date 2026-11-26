//! The logic of the images: the square PNG of an upload, the new icon of a resource, and the read
//! of an image.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::images::{Icon, IconChange, IconGlyph, ImageId, Upload};
use scylla_extension::Tx;
use sqlx::PgPool;

use super::{raster, store};
use crate::Error;

/// A new icon of a resource, with the PNG of a new image, ready for its transaction.
pub(crate) enum NewIcon {
    /// The default icon.
    Default,
    /// A glyph with a color.
    Glyph(IconGlyph),
    /// A new image.
    Png(Vec<u8>),
}

/// Makes the square PNG of an upload on a blocking thread, as the decode takes time. A file that
/// the server cannot use gives `INVALID_ARGUMENT` on `field`.
pub(crate) async fn square_png(upload: Upload, field: &'static str) -> Result<Vec<u8>, Error> {
    let bytes = upload.into_bytes();
    tokio::task::spawn_blocking(move || raster::square_png(&bytes))
        .await
        .map_err(|error| Error::Internal(format!("the decode of an image stopped: {error}")))?
        .map_err(|error| Error::invalid(field, error.to_string()))
}

/// Reads a change of an icon before its transaction: a new image becomes its PNG. A file that
/// the server cannot use gives `INVALID_ARGUMENT` on `field`.
pub(crate) async fn new_icon(change: IconChange, field: &'static str) -> Result<NewIcon, Error> {
    Ok(match change {
        IconChange::Default => NewIcon::Default,
        IconChange::Glyph(glyph) => NewIcon::Glyph(glyph),
        IconChange::Image(upload) => NewIcon::Png(square_png(upload, field).await?),
    })
}

/// Keeps the PNG of a new icon in a transaction, and returns the icon to set. `None` is the
/// default icon.
pub(crate) async fn keep_icon(
    tx: &mut Tx<'_>,
    created_by: ActorId,
    icon: NewIcon,
) -> Result<Option<Icon>, Error> {
    Ok(match icon {
        NewIcon::Default => None,
        NewIcon::Glyph(glyph) => Some(Icon::Glyph(glyph)),
        NewIcon::Png(png) => Some(Icon::Image(store::insert(tx, created_by, &png).await?)),
    })
}

/// Returns the PNG of an image, or `None` if no image has this id.
pub(super) async fn find(pool: &PgPool, id: ImageId) -> Result<Option<Vec<u8>>, Error> {
    let mut tx = pool.begin().await?;
    let data = store::find(&mut tx, id).await?;
    tx.commit().await?;
    Ok(data)
}
