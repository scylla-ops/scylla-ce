//! The check of a route permission on an object of a project: a pipeline or a job. The projects
//! of a workspace that a caller sees in a list.

use scylla_domain::authz::{Object, Permission};
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
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

/// Returns the projects of a workspace whose objects a caller sees in a list: `None` for each
/// project when it has the route permission on the workspace (an owner, an instance admin or a
/// role of the workspace), else the projects where a role gives it the route permission. It is
/// not a route check. A failed check or lookup gives fewer projects and a warning in the log.
pub(crate) async fn visible_projects(
    gate: &dyn Gate,
    caller: &Principal,
    workspace: WorkspaceId,
    holder: Permission<Workspace>,
    access: Permission<Project>,
) -> Option<Vec<ProjectId>> {
    // Give each project when the route permission is on the workspace.
    let object = Object::of(workspace);
    let names = [holder.name()];
    match gate
        .permissions(caller, &object, Some(workspace), &names)
        .await
    {
        Ok(answers) if answers.first().copied().unwrap_or(false) => return None,
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "a check failed: the list shows the shared projects"),
    }

    // Else give the projects where a role gives the route permission.
    match gate.granted(caller, access).await {
        Ok(projects) => Some(projects),
        Err(error) => {
            tracing::warn!(%error, "a lookup failed: the list shows no project");
            Some(Vec::new())
        }
    }
}
