//! The SQL of the `workspaces` table.

use scylla_domain::tenancy::slugs::Slug;
use scylla_domain::tenancy::workspaces::{
    NewSlug, NewWorkspace, VisibleWorkspaces, Workspace, WorkspaceChange, WorkspaceId,
    WorkspaceName,
};
use scylla_domain::PageRequest;
use scylla_extension::{Rejection, Tx};
use time::OffsetDateTime;

use super::owners;
use crate::Error;

/// The unique constraint of the slugs.
const SLUG_KEY: &str = "workspaces_slug_key";

/// The number of slugs that one read of the slugs in use looks at.
const BATCH: u32 = 20;

/// The largest suffix of a derived slug.
const MAX_SUFFIX: u32 = 100_000;

/// Inserts a workspace and its first owner, and returns the workspace. A chosen slug in use
/// gives `SLUG_IN_USE`. A derived slug in use gets the first free suffix: `-2`, `-3` and on.
pub(crate) async fn insert(tx: &mut Tx<'_>, new: &NewWorkspace) -> Result<Workspace, Error> {
    // Insert the workspace with its slug.
    let id = WorkspaceId::generate();
    let (slug, created_at) = match &new.slug {
        NewSlug::Chosen(slug) => {
            let created_at = try_insert(tx, id, &new.name, slug)
                .await?
                .ok_or_else(slug_in_use)?;
            (slug.clone(), created_at)
        }
        NewSlug::Derived(base) => insert_derived(tx, id, &new.name, base).await?,
    };

    // Make the creator its first owner.
    owners::add_owner(tx, id, new.creator).await?;
    Ok(Workspace {
        id,
        name: new.name.as_str().to_owned(),
        slug: slug.as_str().to_owned(),
        created_at,
        personal_tokens_enabled: true,
    })
}

/// Inserts a workspace with the first free slug of `base`, `base-2`, `base-3` and on, and
/// returns the slug and the creation time. A reserved slug is never free.
async fn insert_derived(
    tx: &mut Tx<'_>,
    id: WorkspaceId,
    name: &WorkspaceName,
    base: &Slug,
) -> Result<(Slug, OffsetDateTime), Error> {
    for first in (1..MAX_SUFFIX).step_by(BATCH as usize) {
        // Read which slugs of the batch are in use. A reserved slug is out of the batch.
        let batch: Vec<Slug> = (first..first + BATCH)
            .map(|number| match number {
                1 => base.clone(),
                number => base.with_suffix(number),
            })
            .filter(|slug| !slug.is_reserved())
            .collect();
        let texts: Vec<String> = batch.iter().map(|slug| slug.as_str().to_owned()).collect();
        let used = sqlx::query_scalar!("SELECT slug FROM workspaces WHERE slug = ANY($1)", &texts)
            .fetch_all(&mut **tx)
            .await?;

        // Insert the first free one. A slug that a transaction takes first is in use here.
        for slug in batch
            .iter()
            .filter(|slug| !used.iter().any(|u| u == slug.as_str()))
        {
            if let Some(created_at) = try_insert(tx, id, name, slug).await? {
                return Ok((slug.clone(), created_at));
            }
        }
    }
    Err(Error::Internal(format!(
        "no free slug for {}",
        base.as_str()
    )))
}

/// Inserts a workspace, and returns its creation time, or `None` if the slug is in use.
async fn try_insert(
    tx: &mut Tx<'_>,
    id: WorkspaceId,
    name: &WorkspaceName,
    slug: &Slug,
) -> Result<Option<OffsetDateTime>, Error> {
    let created_at = sqlx::query_scalar!(
        "INSERT INTO workspaces (id, name, slug) VALUES ($1, $2, $3)
         ON CONFLICT (slug) DO NOTHING
         RETURNING created_at",
        id.to_db(),
        name.as_str(),
        slug.as_str(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(created_at)
}

/// Returns a workspace, or `None` if no workspace has this id.
pub(crate) async fn find(tx: &mut Tx<'_>, id: WorkspaceId) -> Result<Option<Workspace>, Error> {
    let row = sqlx::query_as!(
        Row,
        "SELECT id, name, slug, created_at, personal_tokens_enabled FROM workspaces WHERE id = $1",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(Row::into_workspace))
}

/// Returns the workspace of a slug, or `None` if no workspace has this slug.
pub(crate) async fn find_by_slug(tx: &mut Tx<'_>, slug: &Slug) -> Result<Option<Workspace>, Error> {
    let row = sqlx::query_as!(
        Row,
        "SELECT id, name, slug, created_at, personal_tokens_enabled FROM workspaces WHERE slug = $1",
        slug.as_str()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(Row::into_workspace))
}

/// Returns the id if a workspace has it, else `None`.
pub(crate) async fn exists(
    tx: &mut Tx<'_>,
    id: WorkspaceId,
) -> Result<Option<WorkspaceId>, sqlx::Error> {
    let id = sqlx::query_scalar!("SELECT id FROM workspaces WHERE id = $1", id.to_db())
        .fetch_optional(&mut **tx)
        .await?;
    Ok(id.map(WorkspaceId::from_db))
}

/// Returns the workspaces that a caller can read, after the page id, in id order, with one
/// extra row.
pub(crate) async fn list_visible(
    tx: &mut Tx<'_>,
    visible: &VisibleWorkspaces,
    page: PageRequest<WorkspaceId>,
) -> Result<Vec<Workspace>, Error> {
    let granted: Vec<_> = visible.granted.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query_as!(
        Row,
        "SELECT id, name, slug, created_at, personal_tokens_enabled FROM workspaces
         WHERE ($5
                OR id IN (SELECT workspace_id FROM workspace_owners WHERE user_id = $1)
                OR id = ANY($2))
           AND ($3::uuid IS NULL OR id > $3)
         ORDER BY id
         LIMIT $4",
        visible.caller.to_db(),
        &granted,
        page.after.map(WorkspaceId::to_db),
        page.size.get() as i64 + 1,
        visible.all,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(Row::into_workspace).collect())
}

/// Changes the name, the slug or the personal token switch of a workspace, and returns it, or `None` if no workspace has
/// this id. A slug in use gives `SLUG_IN_USE`.
pub(crate) async fn update(
    tx: &mut Tx<'_>,
    id: WorkspaceId,
    change: &WorkspaceChange,
) -> Result<Option<Workspace>, Error> {
    let row = sqlx::query_as!(
        Row,
        "UPDATE workspaces SET name = coalesce($2, name), slug = coalesce($3, slug),
             personal_tokens_enabled = coalesce($4, personal_tokens_enabled)
         WHERE id = $1
         RETURNING id, name, slug, created_at, personal_tokens_enabled",
        id.to_db(),
        change.name.as_ref().map(WorkspaceName::as_str),
        change.slug.as_ref().map(Slug::as_str),
        change.personal_tokens_enabled,
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.constraint() == Some(SLUG_KEY) => slug_in_use(),
        error => error.into(),
    })?;
    Ok(row.map(Row::into_workspace))
}

/// Locks the row of a workspace until the end of the transaction. Returns `false` if no
/// workspace has this id.
pub(crate) async fn lock(tx: &mut Tx<'_>, id: WorkspaceId) -> Result<bool, Error> {
    let row = sqlx::query_scalar!(
        "SELECT id FROM workspaces WHERE id = $1 FOR UPDATE",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}

/// Returns `true` if a workspace holds a project or an App.
pub(crate) async fn has_content(tx: &mut Tx<'_>, workspace: WorkspaceId) -> Result<bool, Error> {
    let found = sqlx::query_scalar!(
        r#"SELECT (EXISTS (SELECT 1 FROM projects WHERE workspace_id = $1)
                   OR EXISTS (SELECT 1 FROM apps WHERE workspace_id = $1)) AS "found!""#,
        workspace.to_db()
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(found)
}

/// Deletes workspaces, with their owners and their invitations.
pub(crate) async fn delete_all(tx: &mut Tx<'_>, ids: &[WorkspaceId]) -> Result<(), Error> {
    let ids: Vec<_> = ids.iter().map(|id| id.to_db()).collect();
    crate::features::tenancy::grants::store::delete_on_scopes(tx, &ids)
        .await
        .map_err(|error| match error {
            crate::Error::Database(error) => error,
            error => sqlx::Error::Protocol(error.to_string()),
        })?;
    sqlx::query!("DELETE FROM workspaces WHERE id = ANY($1)", &ids)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Returns the workspaces that a caller can read and that refuse the personal tokens, in id
/// order.
pub(crate) async fn refusing_personal_tokens(
    tx: &mut Tx<'_>,
    visible: &VisibleWorkspaces,
) -> Result<Vec<WorkspaceId>, sqlx::Error> {
    let granted: Vec<_> = visible.granted.iter().map(|id| id.to_db()).collect();
    let ids = sqlx::query_scalar!(
        "SELECT id FROM workspaces
         WHERE NOT personal_tokens_enabled
           AND (id IN (SELECT workspace_id FROM workspace_owners WHERE user_id = $1)
                OR id = ANY($2))
         ORDER BY id",
        visible.caller.to_db(),
        &granted,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(ids.into_iter().map(WorkspaceId::from_db).collect())
}

/// A row of the `workspaces` table.
struct Row {
    /// The id.
    id: uuid::Uuid,
    /// The display name.
    name: String,
    /// The slug.
    slug: String,
    /// The creation time.
    created_at: OffsetDateTime,
    /// The personal token switch.
    personal_tokens_enabled: bool,
}

impl Row {
    /// Makes the workspace of the row.
    fn into_workspace(self) -> Workspace {
        Workspace {
            id: WorkspaceId::from_db(self.id),
            name: self.name,
            slug: self.slug,
            created_at: self.created_at,
            personal_tokens_enabled: self.personal_tokens_enabled,
        }
    }
}

/// The refusal of a slug that another workspace uses.
fn slug_in_use() -> Error {
    Error::Rejected(Rejection::new(
        "SLUG_IN_USE",
        "another workspace uses this slug",
    ))
}
