//! The SQL of the `fragments` and `fragment_favorites` tables. Each function runs in the
//! transaction of the caller.

use std::collections::HashSet;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::NodeId;
use scylla_domain::tenancy::library::{Author, Fragment, FragmentId, NewFragment};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageRequest;
use scylla_extension::{Rejection, Tx};
use time::OffsetDateTime;
use uuid::Uuid;

use super::origins;
use crate::Error;

/// The unique key of the name of a fragment in its workspace.
const NAME_KEY: &str = "fragments_name_key";

/// The foreign key from a fragment to its workspace.
const WORKSPACE_FOREIGN_KEY: &str = "fragments_workspace_id_fkey";

/// Inserts a fragment with no version. A name that the workspace has gives
/// `FRAGMENT_NAME_IN_USE`, and an unknown workspace `NotFound("workspace")`.
pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    id: FragmentId,
    new: &NewFragment,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO fragments (id, workspace_id, created_by, name, description, tags, origin)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        id.to_db(),
        new.workspace_id.to_db(),
        new.created_by.to_db(),
        new.name.as_str(),
        new.description,
        &new.tags,
        new.origin
            .as_ref()
            .map(|origin| origins::new_json(origin, OffsetDateTime::now_utc())),
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.constraint() == Some(NAME_KEY) => {
            Error::Rejected(Rejection::new(
                "FRAGMENT_NAME_IN_USE",
                "the workspace has a fragment of this name",
            ))
        }
        sqlx::Error::Database(db) if db.constraint() == Some(WORKSPACE_FOREIGN_KEY) => {
            Error::NotFound("workspace")
        }
        error => error.into(),
    })?;
    Ok(())
}

/// Returns a fragment with no version, no favorite and no usage, or `None` if no fragment has
/// this id. `lock` locks its row until the end of the transaction.
pub(crate) async fn find(
    tx: &mut Tx<'_>,
    id: FragmentId,
    lock: bool,
) -> Result<Option<Fragment>, Error> {
    let row = if lock {
        sqlx::query_as!(
            Row,
            r#"SELECT f.id, f.workspace_id, f.created_by, u.display_name AS "author_name?",
                      f.name, f.description, f.tags, f.created_at, f.origin
               FROM fragments f LEFT JOIN users u ON u.id = f.created_by
               WHERE f.id = $1 FOR UPDATE OF f"#,
            id.to_db(),
        )
        .fetch_optional(&mut **tx)
        .await?
    } else {
        sqlx::query_as!(
            Row,
            r#"SELECT f.id, f.workspace_id, f.created_by, u.display_name AS "author_name?",
                      f.name, f.description, f.tags, f.created_at, f.origin
               FROM fragments f LEFT JOIN users u ON u.id = f.created_by
               WHERE f.id = $1"#,
            id.to_db(),
        )
        .fetch_optional(&mut **tx)
        .await?
    };
    row.map(Row::into_fragment).transpose()
}

/// Returns the fragments of a workspace after the page id, in id order, with one extra row.
pub(crate) async fn list(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    page: PageRequest<FragmentId>,
) -> Result<Vec<Fragment>, Error> {
    let rows = sqlx::query_as!(
        Row,
        r#"SELECT f.id, f.workspace_id, f.created_by, u.display_name AS "author_name?",
                  f.name, f.description, f.tags, f.created_at, f.origin
           FROM fragments f LEFT JOIN users u ON u.id = f.created_by
           WHERE f.workspace_id = $1 AND ($2::uuid IS NULL OR f.id > $2)
           ORDER BY f.id
           LIMIT $3"#,
        workspace.to_db(),
        page.after.map(FragmentId::to_db),
        page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter().map(Row::into_fragment).collect()
}

/// Deletes a fragment with its versions and its favorites. The links of the pipelines stay.
pub(crate) async fn delete(tx: &mut Tx<'_>, id: FragmentId) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM fragments WHERE id = $1", id.to_db())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Marks a fragment as a favorite of an actor, or removes the mark.
pub(crate) async fn set_favorite(
    tx: &mut Tx<'_>,
    id: FragmentId,
    actor: ActorId,
    favorite: bool,
) -> Result<(), sqlx::Error> {
    if favorite {
        sqlx::query!(
            "INSERT INTO fragment_favorites (fragment_id, actor_id) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
            id.to_db(),
            actor.to_db(),
        )
        .execute(&mut **tx)
        .await?;
    } else {
        sqlx::query!(
            "DELETE FROM fragment_favorites WHERE fragment_id = $1 AND actor_id = $2",
            id.to_db(),
            actor.to_db(),
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Returns the fragments, among `ids`, that an actor marked as a favorite.
pub(crate) async fn favorites(
    tx: &mut Tx<'_>,
    ids: &[Uuid],
    actor: ActorId,
) -> Result<HashSet<Uuid>, sqlx::Error> {
    let rows = sqlx::query_scalar!(
        "SELECT fragment_id FROM fragment_favorites WHERE fragment_id = ANY($1) AND actor_id = $2",
        ids,
        actor.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().collect())
}

/// A row of the `fragments` table, with the display name of its author.
struct Row {
    /// The id.
    id: Uuid,
    /// The workspace.
    workspace_id: Uuid,
    /// The author.
    created_by: Uuid,
    /// The display name of the author, or `None` for an actor that is not a user.
    author_name: Option<String>,
    /// The name.
    name: String,
    /// The description.
    description: String,
    /// The tags.
    tags: Vec<String>,
    /// The creation time.
    created_at: OffsetDateTime,
    /// The origin in JSON, or `None`.
    origin: Option<serde_json::Value>,
}

impl Row {
    /// Makes the fragment of the row, with no version, no favorite and no usage.
    fn into_fragment(self) -> Result<Fragment, Error> {
        let name = NodeId::parse(self.name)
            .map_err(|error| Error::Internal(format!("bad name of a fragment: {error}")))?;
        Ok(Fragment {
            id: FragmentId::from_db(self.id),
            workspace_id: WorkspaceId::from_db(self.workspace_id),
            author: Author {
                id: ActorId::from_db(self.created_by),
                name: self.author_name.unwrap_or_default(),
            },
            name,
            description: self.description,
            tags: self.tags,
            created_at: self.created_at,
            versions: Vec::new(),
            favorite: false,
            usages: Vec::new(),
            origin: self.origin.map(origins::read).transpose()?,
        })
    }
}
