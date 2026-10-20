pub mod commands;
pub mod queries;
pub mod repository;

pub use commands::{CreateProject, DeleteProject, NewProject, SetProjectActive, UpdateProject};
pub use queries::{
    GetProject, ListOrganizationProjects, ListProjectMembers, ListProjects, ListUserProjects,
};
pub use repository::ProjectRepository;

use crate::application::{DispatchUseCases, UserRepository};
use derive_more::Constructor;
use scylla_auth::authz::VisibilityResolver;
use std::sync::Arc;

/// The project aggregate's stage runners, one block per action in `commands.rs` and
/// `queries.rs`. It has no method of its own; `Actions::run` drives it. A delete stops the
/// live jobs of the project through `dispatch`.
#[derive(Constructor)]
pub struct ProjectUseCases {
    pub(super) project_repo: Arc<dyn ProjectRepository>,
    pub(super) user_repo: Arc<dyn UserRepository>,
    pub(super) visibility: Arc<dyn VisibilityResolver>,
    pub(super) dispatch: Arc<DispatchUseCases>,
}

#[cfg(test)]
mod tests;
