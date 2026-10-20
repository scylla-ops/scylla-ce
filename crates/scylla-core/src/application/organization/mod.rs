pub mod commands;
pub mod queries;
pub mod repository;

pub use commands::{
    CreateOrganization, DeleteOrganization, NewOrganization, SetOrganizationActive,
    UpdateOrganization,
};
pub use queries::{
    GetOrganization, ListOrganizationMembers, ListOrganizations, ListUserOrganizations,
};
pub use repository::OrganizationRepository;

use crate::application::{AppRepository, DispatchUseCases, UserRepository};
use derive_more::Constructor;
use std::sync::Arc;

/// The organization aggregate's stage runners, one block per action in `commands.rs` and
/// `queries.rs`. It has no method of its own; `Actions::run` drives it. A delete stops the
/// live jobs of the organization and closes the agent streams of its Apps through `dispatch`.
#[derive(Constructor)]
pub struct OrganizationUseCases {
    pub(super) org_repo: Arc<dyn OrganizationRepository>,
    pub(super) user_repo: Arc<dyn UserRepository>,
    pub(super) app_repo: Arc<dyn AppRepository>,
    pub(super) dispatch: Arc<DispatchUseCases>,
}

#[cfg(test)]
mod tests;
