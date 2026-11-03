//! One project and an actor for each way to reach it, for the tests of the route permissions.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_testkit::{text, TestApp};
use tonic::{Code, Status};

/// A project in the personal workspace of its owner, and one actor for each role.
pub(super) struct Cast {
    /// The id of the project, as text.
    pub(super) project: String,
    /// The owner of the workspace, and each actor with its role, in a fixed order.
    pub(super) actors: [(&'static str, ActorId); 8],
}

impl Cast {
    /// Makes the project, then gives each role: in the workspace, on the project, or none.
    pub(super) async fn new(app: &TestApp) -> Self {
        // Make the owner, its project, and one user for each role.
        let owner = app.user().await;
        let workspace = app.workspace_of(owner).await;
        let project = app.create_project(owner, "alpha").await.unwrap();
        let project_id = ProjectId::parse(text(&project.id)).unwrap();
        let mut users = Vec::new();
        for _ in 0..7 {
            users.push(app.user().await);
        }

        // Give the roles.
        app.grant(&[
            Workspace::ADMIN.of(workspace, users[0]),
            Workspace::MEMBER.of(workspace, users[1]),
            Workspace::VIEWER.of(workspace, users[2]),
            Project::ADMIN.of(project_id, users[3]),
            Project::EDITOR.of(project_id, users[4]),
            Project::VIEWER.of(project_id, users[5]),
        ])
        .await;
        Self {
            project: project_id.to_string(),
            actors: [
                ("owner", owner),
                ("workspace admin", users[0]),
                ("member", users[1]),
                ("workspace viewer", users[2]),
                ("project admin", users[3]),
                ("editor", users[4]),
                ("project viewer", users[5]),
                ("stranger", users[6]),
            ],
        }
    }

    /// Returns the owner of the workspace.
    pub(super) fn owner(&self) -> ActorId {
        self.actors[0].1
    }
}

/// Returns the roles of the calls that pass. Each other call must get `PERMISSION_DENIED`.
pub(super) fn passed<T>(results: Vec<(&'static str, Result<T, Status>)>) -> Vec<&'static str> {
    results
        .into_iter()
        .filter_map(|(role, result)| match result {
            Ok(_) => Some(role),
            Err(status) => {
                assert_eq!(status.code(), Code::PermissionDenied, "{role}: {status:?}");
                None
            }
        })
        .collect()
}
