//! The proto value of the GitHub App that the server uses, with its webhook URL on this server
//! and its last webhook. No value holds a key or a secret.

use scylla_proto::integrations::v1::VcsProvider;
use scylla_proto::platform::v1 as pb;

use super::service::{Found, Shown};
use crate::api::parse::{self, ToProto};
use crate::features::integrations::IntegrationContext;

/// Makes the proto value of an App.
pub(super) fn app(ctx: &IntegrationContext, shown: Shown) -> pb::VcsApp {
    // Find the webhook URL of the App on this server.
    let api_url = ctx.urls.api_url.trim_end_matches('/');
    let webhook_url = match shown.found.row() {
        Some(id) => format!("{api_url}/integrations/github/{id}/webhook"),
        None => format!("{api_url}/integrations/github/webhook"),
    };

    // Add the last webhook.
    let (last_delivery_at, last_delivery_status) = match shown.last {
        Some(last) => (
            Some(parse::timestamp(last.received_at)),
            last.status
                .and_then(|status| u32::try_from(status).ok())
                .unwrap_or_default(),
        ),
        None => (None, 0),
    };
    let base = pb::VcsApp {
        provider: VcsProvider::Github as i32,
        webhook_url,
        last_delivery_at,
        last_delivery_status,
        ..pb::VcsApp::default()
    };

    // Add what the App of the settings or of the instance tells.
    match shown.found {
        Found::Settings(app_id, slug) => pb::VcsApp {
            app_id: app_id.to_string(),
            web_url: format!("{}/apps/{slug}", ctx.settings.github_web_url),
            name: slug.clone(),
            slug,
            managed_by_settings: true,
            ..base
        },
        Found::Instance(app) => pb::VcsApp {
            id: Some(app.id.to_proto()),
            app_id: app.app_id.to_string(),
            slug: app.slug,
            name: app.name,
            owner: app.owner.unwrap_or_default(),
            web_url: app.web_url,
            created_by: Some(app.created_by.to_proto()),
            created_at: Some(parse::timestamp(app.created_at)),
            ..base
        },
    }
}
