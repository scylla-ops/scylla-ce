//! The logic of the images: the square PNG of an upload, and the read of an image.

use scylla_domain::platform::images::{ImageId, Upload};
use sqlx::PgPool;

use super::{raster, store};
use crate::Error;

/// Makes the square PNG of an upload on a blocking thread, as the decode takes time. A file that
/// the server cannot use gives `INVALID_ARGUMENT` on `field`.
pub(crate) async fn square_png(upload: Upload, field: &'static str) -> Result<Vec<u8>, Error> {
    let bytes = upload.into_bytes();
    tokio::task::spawn_blocking(move || raster::square_png(&bytes))
        .await
        .map_err(|error| Error::Internal(format!("the decode of an image stopped: {error}")))?
        .map_err(|error| Error::invalid(field, error.to_string()))
}

/// Returns the PNG of an image, or `None` if no image has this id.
pub(super) async fn find(pool: &PgPool, id: ImageId) -> Result<Option<Vec<u8>>, Error> {
    let mut tx = pool.begin().await?;
    let data = store::find(&mut tx, id).await?;
    tx.commit().await?;
    Ok(data)
}
