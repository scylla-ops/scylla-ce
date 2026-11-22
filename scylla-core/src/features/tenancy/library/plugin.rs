//! The plugin of the library.

use scylla_domain::tenancy::library::{FragmentCaller, FragmentHolder};
use scylla_extension::{EditionBuilder, Plugin};

use super::grpc;

/// Adds the permission model of the library and `FragmentService`. Install it after
/// `Workspaces`: it extends `Workspace`.
pub(crate) struct Library;

impl Plugin for Library {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition
            .authz::<FragmentHolder>()
            .authz::<FragmentCaller>()
            .service(|routes, ctx| {
                routes.add_service(grpc::server(ctx.pool(), ctx.hooks(), ctx.gate()));
            })
    }
}
