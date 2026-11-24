//! The plugin of the roles feature.

use scylla_domain::tenancy::roles::{RoleAdmin, RoleCaller, RoleHolder};
use scylla_extension::{EditionBuilder, Plugin};

use super::grpc;

/// Adds `RoleService` and the permissions of its routes. The tables come from the core
/// migrations. Install it after `Workspaces`.
pub(crate) struct Roles;

impl Plugin for Roles {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition
            .authz::<RoleAdmin>()
            .authz::<RoleHolder>()
            .authz::<RoleCaller>()
            .service(|routes, ctx| {
                let schema = ctx.authz_model().clone();
                let (grants, hooks) = (ctx.grants(), ctx.hooks());
                routes.add_service(grpc::server(ctx.pool(), ctx.gate(), schema, grants, hooks));
            })
    }
}
