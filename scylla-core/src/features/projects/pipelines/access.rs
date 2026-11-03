//! The check of a route permission on an object of a project: a pipeline or a job.

use scylla_domain::authz::Permission;
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{AccessError, Gate, Principal};

use crate::Error;

/// Checks a route permission on the project of an object and on its workspace, and returns the
/// place of the object. An unknown object and a refusal get the same answer, which names the
/// object, not its project.
pub(crate) async fn authorize(
    gate: &dyn Gate,
    caller: &Principal,
    permission: Permission<Project>,
    object: String,
    place: Option<(ProjectId, WorkspaceId)>,
) -> Result<(ProjectId, WorkspaceId), Error> {
    // Refuse an unknown object.
    let reason = format!("{} on {object} is denied", permission.name());
    let Some((project, workspace)) = place else {
        let error = AccessError::refused(caller, permission.name(), &object, reason);
        return Err(error.into());
    };

    // Check the permission on the project, and hide the project in a refusal.
    match gate
        .authorize(caller, permission.on(project).within(workspace))
        .await
    {
        Ok(()) => Ok((project, workspace)),
        Err(AccessError::Denied(_)) => Err(AccessError::Denied(reason).into()),
        Err(error) => Err(error.into()),
    }
}
