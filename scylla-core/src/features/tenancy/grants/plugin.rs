//! The plugin of the grants feature.

use scylla_domain::tenancy::grants::AccessCaller;
use scylla_extension::{EditionBuilder, Plugin};

use super::grpc;

/// Adds `AccessService`. It gives and lists only the roles of the edition registry, and checks
/// the permissions of the model.
pub(crate) struct Grants;

impl Plugin for Grants {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition.authz::<AccessCaller>().service(|routes, ctx| {
            let schema = ctx.authz_model().clone();
            let (pool, gate, grants) = (ctx.pool(), ctx.gate(), ctx.grants());
            routes.add_service(grpc::server(pool, gate, grants, schema, ctx.hooks()));
        })
    }
}
