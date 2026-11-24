//! What a reader sees of the feed of a workspace: the areas of the whole workspace, and the areas
//! of the projects where a role gives their read permission.

use scylla_domain::activity::{Area, Visible};
use scylla_domain::authz::Object;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{Gate, Principal};

/// Returns the areas and the projects of a workspace in which a reader sees the rows. An area
/// whose read permission is on the workspace gives each of its rows. Else an area of a project
/// gives the rows of the projects where a role gives its read permission. It is not a route check.
/// A failed check or lookup gives fewer rows and a warning in the log.
pub(super) async fn visible(
    gate: &dyn Gate,
    caller: &Principal,
    workspace: WorkspaceId,
) -> Visible {
    // Find the areas whose read permission the reader has on the workspace.
    let object = Object::of(workspace);
    let names: Vec<&'static str> = Area::ALL.iter().map(|area| area.holder().name()).collect();
    let answers = match gate
        .permissions(caller, &object, Some(workspace), &names)
        .await
    {
        Ok(answers) => answers,
        Err(error) => {
            tracing::warn!(%error, "a check failed: the feed shows the shared projects");
            vec![false; names.len()]
        }
    };
    let mut visible = Visible::default();
    for (area, held) in Area::ALL.iter().copied().zip(answers) {
        if held {
            visible.areas.push(area);
        }
    }

    // Find the projects of each other area of a project.
    for area in Area::ALL.iter().copied() {
        let Some(access) = area.access().filter(|_| !visible.areas.contains(&area)) else {
            continue;
        };
        match gate.granted(caller, access).await {
            Ok(projects) => visible
                .projects
                .extend(projects.into_iter().map(|project| (project, area))),
            Err(error) => {
                tracing::warn!(%error, area = area.as_str(), "a lookup failed: the feed hides it");
            }
        }
    }
    visible
}
