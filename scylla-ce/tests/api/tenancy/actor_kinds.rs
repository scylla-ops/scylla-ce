//! The tests of the kind of an actor in `ListMembers` and `ListRoleHolders`: a user, an agent
//! or another App.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_testkit::{text, Role, TestApp};
use sqlx::PgPool;

use crate::app;

/// Makes an App in a workspace and gives it the role runner there. Returns its id.
async fn app_with_runner_role(
    app: &TestApp,
    owner: ActorId,
    workspace: WorkspaceId,
    runner: bool,
) -> String {
    let created = app.create_app(owner, workspace, runner).await.unwrap();
    let id = text(&created.app.unwrap().id).to_owned();
    app.grant_role(
        owner,
        Role::on("workspace", &workspace.to_string(), "runner"),
        &id,
    )
    .await
    .unwrap();
    id
}

/// A member tells if it is an App that runs jobs. A user and another App are not agents.
#[sqlx::test(migrations = false)]
async fn list_members_tells_an_agent_from_another_app(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let agent = app_with_runner_role(&app, owner, workspace, true).await;
    let ci = app_with_runner_role(&app, owner, workspace, false).await;

    // when
    let members = app
        .members_of(owner, "workspace", &workspace.to_string())
        .await
        .unwrap();

    // then
    let runner = |id: &str| {
        let member = members.iter().find(|member| text(&member.actor_id) == id);
        member.map(|member| (member.actor_kind.as_str(), member.runner))
    };
    assert_eq!(runner(&agent), Some(("app", true)));
    assert_eq!(runner(&ci), Some(("app", false)));
    assert_eq!(runner(&owner.to_string()), Some(("user", false)));
}

/// A holder of a role tells if it is an App that runs jobs.
#[sqlx::test(migrations = false)]
async fn list_role_holders_tells_an_agent_from_another_app(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let agent = app_with_runner_role(&app, owner, workspace, true).await;
    let ci = app_with_runner_role(&app, owner, workspace, false).await;

    // when
    let holders = app
        .role_holders(owner, "agent", Some(&workspace.to_string()))
        .await
        .unwrap();

    // then
    let runner = |id: &str| {
        let holder = holders
            .holders
            .iter()
            .find(|holder| text(&holder.actor_id) == id);
        holder.map(|holder| holder.runner)
    };
    assert_eq!(holders.total, 2);
    assert_eq!(runner(&agent), Some(true));
    assert_eq!(runner(&ci), Some(false));
}
