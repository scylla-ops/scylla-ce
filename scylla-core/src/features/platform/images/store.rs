//! The SQL of the `images` table. Each function runs in the transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::images::{Image, ImageId};
use scylla_extension::Tx;

use crate::Error;

/// Keeps the PNG of an upload, and returns the id of its new row.
pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    created_by: ActorId,
    png: &[u8],
) -> Result<ImageId, Error> {
    let id = ImageId::generate();
    sqlx::query!(
        "INSERT INTO images (id, created_by, content_type, data) VALUES ($1, $2, $3, $4)",
        id.to_db(),
        created_by.to_db(),
        Image::CONTENT_TYPE,
        png,
    )
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

/// Deletes an image. A resource that still shows it gets no image: its foreign key sets null.
pub(crate) async fn delete(tx: &mut Tx<'_>, id: ImageId) -> Result<(), Error> {
    sqlx::query!("DELETE FROM images WHERE id = $1", id.to_db())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Returns the PNG of an image, or `None` if no image has this id.
pub(super) async fn find(tx: &mut Tx<'_>, id: ImageId) -> Result<Option<Vec<u8>>, Error> {
    let data = sqlx::query_scalar!("SELECT data FROM images WHERE id = $1", id.to_db())
        .fetch_optional(&mut **tx)
        .await?;
    Ok(data)
}
