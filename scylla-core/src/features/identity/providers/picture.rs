//! The photo of a person that a provider gives: its address, its download, and its import as the
//! avatar of a new account. A GitHub photo comes only from an avatar host of its GitHub. A failed
//! import at a sign-up goes to the log, and the new user keeps the initials.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::images::{Image, Upload};
use url::Url;

use super::picture_store;
use super::service::{self, Kind};
use super::store::{self, ProviderRow};
use crate::api::web::policy::github_avatars;
use crate::features::identity::accounts::{self, AccountContext};
use crate::features::platform::images;
use crate::infra::download::{self, Reach};
use crate::Error;

/// The longest address of a photo that the server keeps.
const MAX_ADDRESS: usize = 2048;

/// The content types of a photo.
const IMAGE_TYPES: [&str; 3] = ["image/png", "image/jpeg", "image/webp"];

/// Keeps the address of a photo that a provider gave, if it is not empty and not too long.
pub(super) fn address(url: Option<String>) -> Option<String> {
    url.filter(|url| !url.is_empty() && url.len() <= MAX_ADDRESS)
}

/// Returns the provider of an identity of a user, and the address of its photo. An unknown
/// identity gives `NotFound("identity")`.
pub(crate) async fn picture_of(
    ctx: &AccountContext,
    user: ActorId,
    slug: &str,
    subject: &str,
) -> Result<(ProviderRow, Option<String>), Error> {
    let mut tx = ctx.pool.begin().await?;
    let provider = store::by_slug(&mut tx, slug).await?;
    let picture = match &provider {
        Some(provider) => picture_store::picture_of(&mut tx, user, provider.id, subject).await?,
        None => None,
    };
    tx.commit().await?;
    match (provider, picture) {
        (Some(provider), Some(picture)) => Ok((provider, picture)),
        _ => Err(Error::NotFound("identity")),
    }
}

/// Downloads the photo of an identity, and makes the square PNG of an avatar from it, as from an
/// upload. Returns the reason of a failure.
pub(crate) async fn square_png(
    ctx: &AccountContext,
    provider: &ProviderRow,
    url: &str,
) -> Result<Vec<u8>, String> {
    // A GitHub photo must come from an avatar host of its GitHub.
    if let Kind::GitHub(settings) = service::kind(provider).map_err(|error| error.to_string())? {
        let origin = Url::parse(url).map(|url| url.origin().ascii_serialization());
        if !origin.is_ok_and(|origin| github_avatars(&settings.web_url).contains(&origin)) {
            return Err("the photo is not on an avatar host of the GitHub".to_owned());
        }
    }

    // Download the file, then make the square PNG.
    let reach = match ctx.settings.local_pictures {
        true => Reach::Local,
        false => Reach::Public,
    };
    let bytes = download::download(url, reach, &IMAGE_TYPES, Image::MAX_UPLOAD).await;
    let upload = Upload::parse(bytes.map_err(|refusal| refusal.to_string())?);
    let upload = upload.map_err(|error| error.to_string())?;
    let png = images::service::square_png(upload, "picture").await;
    png.map_err(|error| error.to_string())
}

/// Imports the photo of the provider as the avatar of a new user, after the transaction that made
/// it. A failure goes to the log only.
pub(super) async fn import_at_sign_up(
    ctx: &AccountContext,
    provider: &ProviderRow,
    user: ActorId,
    url: Option<&str>,
) {
    let Some(url) = url else {
        return;
    };
    let imported = match square_png(ctx, provider, url).await {
        Ok(png) => accounts::keep_photo(&ctx.pool, user, png, false)
            .await
            .map_err(|error| error.to_string()),
        Err(reason) => Err(reason),
    };
    let slug = &provider.slug;
    match imported {
        Ok(_) => tracing::info!(event = "account.avatar_imported", %user, provider = slug),
        Err(reason) => tracing::warn!(
            event = "account.avatar_import_failed",
            %user,
            provider = slug,
            reason,
            "the photo of the provider was not imported"
        ),
    }
}
