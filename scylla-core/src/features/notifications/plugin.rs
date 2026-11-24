//! The plugin of the notifications.

use scylla_domain::notifications::NotificationCaller;
use scylla_extension::{EditionBuilder, Plugin};

use super::{grpc, kinds};

/// Adds `NotificationService` and the kinds of the core.
pub(crate) struct Notifications;

impl Plugin for Notifications {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        let edition = edition
            .authz::<NotificationCaller>()
            .service(|routes, ctx| {
                let (pool, gate) = (ctx.pool(), ctx.gate());
                routes.add_service(grpc::server(pool, gate, ctx.notification_kinds()));
            });
        kinds::all()
            .into_iter()
            .fold(edition, |edition, kind| edition.notification_kind(kind))
    }
}
