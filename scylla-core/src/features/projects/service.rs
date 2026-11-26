//! The logic of the projects: one transaction for each call, with the hooks.

use scylla_domain::authz::{Object, Resource};
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::platform::images::{Icon, IconChange, ImageId};
use scylla_domain::projects::{
    ListProjects, NewProject, Project, ProjectChange, ProjectId, ProjectPage, Visible, KIND,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Page;
use scylla_extension::{CreateContext, DeleteContext, Gate, Hooks, Tx, WorkspaceFuture};
use sqlx::PgPool;

use super::store;
use crate::features::platform::images;
use crate::features::tenancy::grants;
use crate::Error;

/// Creates a project if each `BeforeCreate` hook accepts it. Its workspace is in its row only.
pub(super) async fn create(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    new: NewProject,
) -> Result<Project, Error> {
    let mut tx = pool.begin().await?;
    hooks
        .before_create(&mut tx, &CreateContext::new(KIND, actor, new.workspace_id))
        .await?;
    let project = store::insert(&mut tx, &new).await?;
    tx.commit().await?;
    Ok(project)
}

/// Returns the workspace of a project, or `None` if no project has this id.
pub(super) async fn workspace(pool: &PgPool, id: ProjectId) -> Result<Option<WorkspaceId>, Error> {
    let mut tx = pool.begin().await?;
    let workspace = store::workspace(&mut tx, id).await?;
    tx.commit().await?;
    Ok(workspace)
}

/// Finds the workspace of a project, for the roles of `AccessService`.
pub(super) fn find_workspace(pool: PgPool, id: ProjectId) -> WorkspaceFuture {
    Box::pin(async move {
        let mut tx = pool.begin().await?;
        let workspace = store::workspace(&mut tx, id).await?;
        tx.commit().await?;
        Ok(workspace)
    })
}

/// Returns a project, or `None` if no project has this id.
pub(super) async fn get(pool: &PgPool, id: ProjectId) -> Result<Option<Project>, Error> {
    let mut tx = pool.begin().await?;
    let project = store::find(&mut tx, id).await?;
    tx.commit().await?;
    Ok(project)
}

/// Returns one page of the projects that a caller can see, oldest first.
pub(super) async fn list(
    pool: &PgPool,
    query: ListProjects,
    visible: Visible,
) -> Result<ProjectPage, Error> {
    let mut tx = pool.begin().await?;
    let projects = store::list_visible(&mut tx, &visible, query.workspace_id, query.page).await?;
    tx.commit().await?;
    Ok(Page::from_overfetch(projects, query.page.size, |project| {
        project.id
    }))
}

/// Changes the name, the description or the icon of a project, and returns it. The last change
/// wins. A new image replaces the old image of the project, which goes.
pub(super) async fn update(
    pool: &PgPool,
    actor: ActorId,
    id: ProjectId,
    mut change: ProjectChange,
) -> Result<Project, Error> {
    // Make the PNG of a new image before the transaction, as the decode takes time.
    let icon = match change.icon.take() {
        None => None,
        Some(IconChange::Default) => Some(NewIcon::Default),
        Some(IconChange::Glyph(glyph)) => Some(NewIcon::Icon(Icon::Glyph(glyph))),
        Some(IconChange::Image(upload)) => Some(NewIcon::Png(
            images::service::square_png(upload, "icon.image").await?,
        )),
    };

    // Change the icon, then the other fields, in one transaction.
    let mut tx = pool.begin().await?;
    if let Some(icon) = icon {
        set_icon(&mut tx, actor, id, icon).await?;
    }
    let project = store::update(&mut tx, id, &change)
        .await?
        .ok_or(Error::NotFound("project"))?;
    tx.commit().await?;
    Ok(project)
}

/// A new icon of a project, with the PNG of a new image.
enum NewIcon {
    /// The default icon.
    Default,
    /// A glyph.
    Icon(Icon),
    /// A new image.
    Png(Vec<u8>),
}

/// Sets the icon of a project in a transaction: keeps the PNG of a new image, and deletes the
/// old image of the project.
async fn set_icon(
    tx: &mut Tx<'_>,
    actor: ActorId,
    id: ProjectId,
    icon: NewIcon,
) -> Result<(), Error> {
    // Lock the row of the project and read its old image.
    let old: Option<ImageId> = store::lock_icon(tx, id)
        .await?
        .ok_or(Error::NotFound("project"))?;

    // Keep a new image, then set the icon.
    let icon = match icon {
        NewIcon::Default => None,
        NewIcon::Icon(icon) => Some(icon),
        NewIcon::Png(png) => Some(Icon::Image(images::store::insert(tx, actor, &png).await?)),
    };
    store::set_icon(tx, id, icon.as_ref()).await?;

    // Delete the old image: no row shows it now.
    if let Some(old) = old {
        images::store::delete(tx, old).await?;
    }
    Ok(())
}

/// Deletes a project with its secrets and the image of its icon, runs the `AfterDelete` hooks,
/// then removes the roles on it.
pub(super) async fn delete(
    pool: &PgPool,
    hooks: &Hooks,
    gate: &dyn Gate,
    actor: Actor,
    id: ProjectId,
) -> Result<(), Error> {
    // Delete the row and run the hooks in one transaction.
    let mut tx = pool.begin().await?;
    let (workspace_id, image) = store::delete(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("project"))?;
    if let Some(image) = image {
        images::store::delete(&mut tx, image).await?;
    }
    grants::store::delete_on_scopes(&mut tx, &[id.to_db()]).await?;
    hooks
        .after_delete(
            &mut tx,
            &DeleteContext::new(KIND, actor, workspace_id)
                .with_object(format!("{}:{id}", Project::NAME)),
        )
        .await?;
    tx.commit().await?;

    // Remove the roles after the commit. A role that stays reaches no project.
    if let Err(error) = gate.delete_object(&Object::of(id)).await {
        tracing::error!(%error, project = %id, "roles stay after a delete");
    }
    Ok(())
}
