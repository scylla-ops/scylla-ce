//! A workspace with projects and pipelines, for the tests of the activity feed.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::activity::v1::Activity;
use scylla_testkit::{exec_node, text, TestApp};

/// The personal workspace of a new owner, with a project `alpha` and its pipeline `build`.
pub(super) struct Place {
    /// The owner of the workspace.
    pub(super) owner: ActorId,
    /// The id of the workspace, as text.
    pub(super) workspace: String,
    /// The id of the project `alpha`, as text.
    pub(super) project: String,
    /// The id of the pipeline of `alpha`, as text.
    pub(super) pipeline: String,
}

impl Place {
    /// Makes the owner, the project `alpha` and its pipeline.
    pub(super) async fn new(app: &TestApp) -> Self {
        let owner = app.user().await;
        let workspace = app.workspace_of(owner).await.to_string();
        let mut place = Self {
            owner,
            workspace,
            project: String::new(),
            pipeline: String::new(),
        };
        (place.project, place.pipeline) = place.add_project(app, "alpha").await;
        place
    }

    /// Makes a project with a pipeline `build` in the workspace. Returns the ids of the project
    /// and of the pipeline.
    pub(super) async fn add_project(&self, app: &TestApp, name: &str) -> (String, String) {
        let project = app.create_project(self.owner, name).await.unwrap();
        let project = text(&project.id).to_owned();
        let pipeline = app
            .create_pipeline(self.owner, &project, vec![exec_node("build", &[])])
            .await
            .unwrap();
        (project, text(&pipeline.id).to_owned())
    }

    /// Starts a job of a pipeline and ends it with a cancel. Returns the id of the job.
    pub(super) async fn ended_job(&self, app: &TestApp, pipeline: &str) -> String {
        let job = app.run_pipeline(self.owner, pipeline).await.unwrap();
        let id = text(&job.id).to_owned();
        app.cancel_job(self.owner, &id).await.unwrap();
        id
    }

    /// Makes a secret in the project `alpha`. Returns its id.
    pub(super) async fn secret(&self, app: &TestApp, name: &str) -> String {
        let response = app
            .create_secret(self.owner, &self.project, name, "hidden-first")
            .await;
        text(&response.unwrap().secret.unwrap().id).to_owned()
    }
}

/// Returns the parameters of a row, in key order.
pub(super) fn params(row: &Activity) -> Vec<(&str, &str)> {
    let mut params: Vec<_> = row
        .params
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    params.sort_unstable();
    params
}

/// Returns the kinds of rows, in their order.
pub(super) fn kinds(rows: &[Activity]) -> Vec<&str> {
    rows.iter().map(|row| row.kind.as_str()).collect()
}
