//! The image of the avatar of a user: the PNG of an upload or of the photo of a provider, the SVG
//! of a glyph avatar, and the delete of the old image.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users::User;
use scylla_domain::platform::images::{GlyphAvatar, Upload};
use scylla_extension::Tx;
use sqlx::PgPool;

use super::{refused, store, AccountContext};
use crate::features::identity::providers;
use crate::features::platform::images;
use crate::Error;

/// A new image of an avatar, ready for its transaction.
pub(crate) enum NewImage {
    /// The PNG of an upload or of the photo of a provider.
    Png(Vec<u8>),
    /// The SVG of a glyph avatar, with its options.
    Glyph(String, GlyphAvatar),
}

/// Makes the PNG of a new image of an avatar, or the SVG of a new glyph avatar, before the
/// transaction, as the decode takes time. `Some(None)` removes the image.
pub(super) async fn prepare(
    upload: Option<Option<Upload>>,
    glyph: Option<GlyphAvatar>,
) -> Result<Option<Option<NewImage>>, Error> {
    if let Some(glyph) = glyph {
        let svg = images::service::glyph_svg(&glyph)?;
        return Ok(Some(Some(NewImage::Glyph(svg, glyph))));
    }
    match upload {
        Some(Some(upload)) => Ok(Some(Some(NewImage::Png(
            images::service::square_png(upload, "avatar_image").await?,
        )))),
        Some(None) => Ok(Some(None)),
        None => Ok(None),
    }
}

/// Sets the image of the avatar of a user, whose row the transaction locked: keeps the new image,
/// then deletes the old image. `None` removes the image.
pub(super) async fn set_image(
    tx: &mut Tx<'_>,
    user: ActorId,
    image: Option<NewImage>,
) -> Result<(), Error> {
    // Keep the new image, and set it.
    let old = store::avatar_image(tx, user).await?;
    let new = match image {
        Some(NewImage::Png(png)) => Some(images::store::insert(tx, user, &png).await?),
        Some(NewImage::Glyph(svg, glyph)) => {
            Some(images::store::insert_glyph(tx, user, &svg, &glyph).await?)
        }
        None => None,
    };
    store::set_avatar_image(tx, user, new).await?;

    // Delete the old image: no row shows it now.
    if let Some(old) = old {
        images::store::delete(tx, old).await?;
    }
    Ok(())
}

/// Makes the square PNG of the photo of a provider the image of the avatar of a user, and returns
/// the user after the change. With `replace` false, an image that the user has stays.
pub(crate) async fn keep_photo(
    pool: &PgPool,
    user: ActorId,
    png: Vec<u8>,
    replace: bool,
) -> Result<User, Error> {
    let mut tx = pool.begin().await?;
    if !store::lock_user(&mut tx, user).await? {
        return Err(Error::NotFound("user"));
    }
    if replace || store::avatar_image(&mut tx, user).await?.is_none() {
        set_image(&mut tx, user, Some(NewImage::Png(png))).await?;
    }
    let changed = store::find_user(&mut tx, user).await?;
    tx.commit().await?;
    changed.ok_or(Error::NotFound("user"))
}

/// Downloads again the photo of an identity of a user at its provider, and makes it the image of
/// the avatar in place of the current one. An unknown identity gives `NotFound("identity")`.
pub(super) async fn import(
    ctx: &AccountContext,
    user: ActorId,
    provider: &str,
    subject: &str,
) -> Result<User, Error> {
    let (row, url) = providers::picture_of(ctx, user, provider, subject).await?;
    let url = url.ok_or_else(|| refused("NO_PICTURE", "the provider gave no photo"))?;
    let png = providers::picture_png(ctx, &row, &url)
        .await
        .map_err(|reason| {
            let slug = &row.slug;
            tracing::warn!(event = "account.avatar_import_failed", %user, provider = slug, reason);
            refused(
                "PICTURE_UNAVAILABLE",
                "the photo of the provider is not available",
            )
        })?;
    let changed = keep_photo(&ctx.pool, user, png, true).await?;
    tracing::info!(event = "account.avatar_imported", %user, provider = row.slug);
    Ok(changed)
}
