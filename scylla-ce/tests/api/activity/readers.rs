//! The readers of each area: a row shows only to a caller with the read permission of its area,
//! on its project or on the workspace. A caller with no role gets an empty list.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_testkit::{Role, TestApp};
use sqlx::PgPool;

use super::place::Place;
use crate::app;

/// Returns the rows that an actor reads, as `<kind> <project>`, newest first.
async fn seen(app: &TestApp, actor: ActorId, place: &Place) -> Vec<String> {
    let rows = app.activity_of(actor, &place.workspace).await;
    rows.iter()
        .map(|row| {
            format!("{} {}", row.kind, row.project_name)
                .trim_end()
                .to_owned()
        })
        .collect()
}

/// Each reader sees the rows of the areas that it reads: the jobs and the triggers with a view
/// of their project, the secrets with an edit, the members with a read of the workspace. A role
/// on another project shows only the rows of that project, and no role shows no row.
#[sqlx::test(migrations = false)]
async fn list_activity_shows_each_area_to_its_readers(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = Place::new(&app).await;
    let owner = place.owner;
    let (beta, beta_pipeline) = place.add_project(&app, "beta").await;
    let workspace = WorkspaceId::parse(&place.workspace).unwrap();
    let (alpha_id, beta_id) = (
        ProjectId::parse(&place.project).unwrap(),
        ProjectId::parse(&beta).unwrap(),
    );
    let mut users = Vec::new();
    for _ in 0..8 {
        users.push(app.user().await);
    }
    app.grant(&[
        Workspace::ADMIN.of(workspace, users[0]),
        Workspace::MEMBER.of(workspace, users[1]),
        Project::ADMIN.of(alpha_id, users[3]),
        Project::EDITOR.of(alpha_id, users[4]),
        Project::VIEWER.of(alpha_id, users[5]),
        Project::ADMIN.of(beta_id, users[6]),
    ])
    .await;

    // when
    place.ended_job(&app, &beta_pipeline).await;
    place.ended_job(&app, &place.pipeline).await;
    let trigger = app
        .create_trigger(
            owner,
            &place.pipeline,
            "nightly",
            Some("0 3 * * *"),
            Vec::new(),
        )
        .await
        .unwrap();
    place.secret(&app, "TOKEN").await;
    let viewer = Role::on("workspace", &place.workspace, "viewer");
    app.grant_role(owner, viewer, &users[2].to_string())
        .await
        .unwrap();
    let mut results = Vec::new();
    for actor in [owner].iter().chain(&users) {
        results.push(seen(&app, *actor, &place).await);
    }

    // then
    assert!(trigger.trigger.is_some());
    let all = [
        "member.role_granted",
        "secret.created alpha",
        "trigger.created alpha",
        "job.ended alpha",
        "job.ended beta",
    ];
    let without = |left: &[&str]| -> Vec<String> {
        all.iter()
            .filter(|row| !left.contains(row))
            .map(|row| (*row).to_owned())
            .collect()
    };
    let members = all[0];
    let secrets = all[1];
    let beta_rows = all[4];
    let expected = [
        without(&[]),
        without(&[]),
        without(&[]),
        without(&[secrets]),
        without(&[members, beta_rows]),
        without(&[members, beta_rows]),
        without(&[members, secrets, beta_rows]),
        vec![beta_rows.to_owned()],
        Vec::new(),
    ];
    assert_eq!(results, expected);
}
