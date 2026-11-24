//! The plugin of the state of a user.

use scylla_domain::identity::user_state::UserStateCaller;
use scylla_extension::{EditionBuilder, Plugin};

use super::grpc;

/// Adds `UserStateService`.
pub(crate) struct UserState;

impl Plugin for UserState {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition.authz::<UserStateCaller>().service(|routes, ctx| {
            routes.add_service(grpc::server(ctx.pool(), ctx.gate()));
        })
    }
}
