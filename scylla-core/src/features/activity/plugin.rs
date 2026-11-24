//! The plugin of the activity feed.

use scylla_domain::activity::ActivityCaller;
use scylla_extension::{EditionBuilder, Plugin};

use super::grpc;

/// Adds the permission model of the feed and `ActivityService`.
pub(crate) struct Activities;

impl Plugin for Activities {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition.authz::<ActivityCaller>().service(|routes, ctx| {
            routes.add_service(grpc::server(ctx.pool(), ctx.gate()));
        })
    }
}
