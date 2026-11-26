//! The logic of the groups of the projects and of their order: one transaction for each call, with
//! the hooks. Each change locks the order of its workspace first.

use scylla_domain::identity::actors::Actor;
use scylla_domain::projects::groups::{
    self, ListProjectGroups, NewProjectGroup, ProjectGroup, ProjectGroupChange, ProjectGroupId,
    ProjectMove, KIND,
};
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Page;
use scylla_extension::{CreateContext, DeleteContext, Hooks};
use sqlx::PgPool;

use super::store;
use crate::features::projects::store as projects;
use crate::Error;

/// Returns one page of the groups of a workspace, in their order.
pub(super) async fn list(
    pool: &PgPool,
    query: ListProjectGroups,
) -> Result<Page<ProjectGroup, ProjectGroupId>, Error> {
    // Read each group of the workspace.
    let mut tx = pool.begin().await?;
    let all = store::list(&mut tx, query.workspace_id).await?;
    tx.commit().await?;

    // Keep the groups after the group of the request, then one page of them.
    let start = match query.page.after {
        None => 0,
        Some(after) => all
            .iter()
            .position(|group| group.id == after)
            .map_or(all.len(), |index| index + 1),
    };
    let items = all.into_iter().skip(start).collect();
    Ok(Page::from_overfetch(items, query.page.size, |group| {
        group.id
    }))
}

/// Returns the workspace of a group, or `None` if no group has this id.
pub(super) async fn workspace(
    pool: &PgPool,
    id: ProjectGroupId,
) -> Result<Option<WorkspaceId>, Error> {
    let mut tx = pool.begin().await?;
    let workspace = store::workspace(&mut tx, id).await?;
    tx.commit().await?;
    Ok(workspace)
}

/// Returns the workspace of a project, or `None` if no project has this id.
pub(super) async fn project_workspace(
    pool: &PgPool,
    id: ProjectId,
) -> Result<Option<WorkspaceId>, Error> {
    let mut tx = pool.begin().await?;
    let workspace = projects::workspace(&mut tx, id).await?;
    tx.commit().await?;
    Ok(workspace)
}

/// Creates a group after the other groups of its workspace, if each `BeforeCreate` hook accepts
/// it.
pub(super) async fn create(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    new: NewProjectGroup,
) -> Result<ProjectGroup, Error> {
    let mut tx = pool.begin().await?;
    hooks
        .before_create(&mut tx, &CreateContext::new(KIND, actor, new.workspace_id))
        .await?;
    store::lock_workspace(&mut tx, new.workspace_id).await?;
    let group = store::insert(&mut tx, &new).await?;
    tx.commit().await?;
    Ok(group)
}

/// Changes the name or the color of a group, or its place in the order of the groups, and
/// returns it.
pub(super) async fn update(
    pool: &PgPool,
    id: ProjectGroupId,
    change: ProjectGroupChange,
) -> Result<ProjectGroup, Error> {
    // Lock the order of the workspace of the group.
    let mut tx = pool.begin().await?;
    let workspace = store::workspace(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("project group"))?;
    store::lock_workspace(&mut tx, workspace).await?;

    // Move the group, then give it its new name and its new color.
    if let Some(position) = change.position {
        let mut order: Vec<_> = store::list(&mut tx, workspace)
            .await?
            .into_iter()
            .map(|group| group.id)
            .collect();
        groups::place_at(&mut order, id, position);
        store::set_order(&mut tx, &order).await?;
    }
    if let Some(name) = &change.name {
        store::rename(&mut tx, id, name.as_str()).await?;
    }
    if let Some(color) = &change.color {
        store::set_color(&mut tx, id, color.as_ref()).await?;
    }
    let group = store::find(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("project group"))?;
    tx.commit().await?;
    Ok(group)
}

/// Deletes a group and runs the `AfterDelete` hooks. Its projects come after the projects with no
/// group, in their order.
pub(super) async fn delete(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    id: ProjectGroupId,
) -> Result<(), Error> {
    // Lock the order of the workspace of the group.
    let mut tx = pool.begin().await?;
    let workspace = store::workspace(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("project group"))?;
    store::lock_workspace(&mut tx, workspace).await?;

    // Put the projects of the group after the projects with no group, then delete the group.
    let mut loose = store::section(&mut tx, workspace, None).await?;
    loose.extend(store::section(&mut tx, workspace, Some(id)).await?);
    store::set_section(&mut tx, None, &loose).await?;
    store::delete(&mut tx, id).await?;

    // Run the hooks in the transaction of the delete.
    hooks
        .after_delete(
            &mut tx,
            &DeleteContext::new(KIND, actor, workspace).with_object(format!("project_group:{id}")),
        )
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Moves a project to a group, or out of all groups, and to a place there, and returns it. A
/// group of another workspace, or the project itself as the project to come before, gives
/// `INVALID_ARGUMENT`.
pub(super) async fn move_project(pool: &PgPool, moved: ProjectMove) -> Result<Project, Error> {
    // Refuse the project itself as the project to come before.
    if moved.before == Some(moved.project_id) {
        return Err(Error::invalid(
            "before_project_id",
            "is the project to move",
        ));
    }

    // Lock the order of the workspace of the project.
    let mut tx = pool.begin().await?;
    let workspace = projects::workspace(&mut tx, moved.project_id)
        .await?
        .ok_or(Error::NotFound("project"))?;
    store::lock_workspace(&mut tx, workspace).await?;

    // Refuse a group of another workspace.
    if let Some(group) = moved.group_id {
        if store::workspace(&mut tx, group).await? != Some(workspace) {
            return Err(Error::invalid(
                "group_id",
                "is not a group of the workspace of the project",
            ));
        }
    }

    // Put the project at its place in the group, then read it.
    let mut order = store::section(&mut tx, workspace, moved.group_id).await?;
    groups::place(&mut order, moved.project_id, moved.before);
    store::set_section(&mut tx, moved.group_id, &order).await?;
    let project = projects::find(&mut tx, moved.project_id)
        .await?
        .ok_or(Error::NotFound("project"))?;
    tx.commit().await?;
    Ok(project)
}
