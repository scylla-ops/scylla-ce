//! The SQL of the custom icon of an App, in the `apps` table.

use scylla_domain::identity::apps::AppId;
use scylla_domain::platform::images::{Icon, ImageId};
use scylla_extension::Tx;

/// Locks the row of an App and returns the image of its icon, or `None` if no App has this id.
pub(crate) async fn lock(
    tx: &mut Tx<'_>,
    id: AppId,
) -> Result<Option<Option<ImageId>>, sqlx::Error> {
    let image = sqlx::query_scalar!(
        "SELECT icon_image_id FROM apps WHERE id = $1 FOR UPDATE",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(image.map(|image| image.map(ImageId::from_db)))
}

/// Sets the custom icon of an App. `None` gives the default icon.
pub(crate) async fn set(
    tx: &mut Tx<'_>,
    id: AppId,
    icon: Option<&Icon>,
) -> Result<(), sqlx::Error> {
    let (name, color, image) = match icon {
        None => (None, None, None),
        Some(Icon::Glyph(glyph)) => (
            Some(glyph.name.as_str()),
            glyph.color.as_ref().map(|color| color.as_str()),
            None,
        ),
        Some(Icon::Image(image) | Icon::GlyphAvatar(image, _)) => (None, None, Some(image.to_db())),
    };
    sqlx::query!(
        "UPDATE apps SET icon_name = $2, icon_color = $3, icon_image_id = $4 WHERE id = $1",
        id.to_db(),
        name,
        color,
        image,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
