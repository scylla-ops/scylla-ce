//! The mount of the checkout of a job on an agent: the task that deletes the GitLab deploy tokens
//! that no job needs.

use tonic::service::RoutesBuilder;

use super::cleanup;
use crate::features::integrations::{IntegrationContext, Part};

/// Adds the task of this part: the cleanup of the deploy tokens.
pub(crate) fn mount(ctx: &IntegrationContext, _services: &mut RoutesBuilder) -> Part {
    Part {
        tasks: vec![cleanup::spawn(ctx.clone())],
        ..Part::default()
    }
}
