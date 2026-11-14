//! The mount of the pipelines from the files of a repository: the public JSON Schema of the
//! files.

use tonic::service::RoutesBuilder;

use super::schema;
use crate::features::integrations::{IntegrationContext, Part};

/// Adds the services, the routes and the tasks of this part: the route of the schema.
pub(crate) fn mount(ctx: &IntegrationContext, _services: &mut RoutesBuilder) -> Part {
    Part {
        router: schema::router(&ctx.urls.api_url),
        tasks: Vec::new(),
    }
}
