//! The logic of the inbox: each route acts on the caller only, and a row that the caller can no
//! longer open is not in the answer.

use std::collections::{HashMap, HashSet};

use scylla_domain::authz::Object;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::notifications::{Filter, Notification, NotificationId, Summary, MAX_IDS};
use scylla_domain::platform::Platform;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::{Page, PageRequest};
use scylla_extension::{Access, Gate, NotificationKinds, Principal, Tx, REASON_SUBJECT};
use sqlx::PgPool;
use uuid::Uuid;

use super::emit::{PROJECT_NAME, WORKSPACE_NAME};
use super::store::inbox;
use crate::Error;

/// The most pages that one list reads to find a row that the caller can see.
const MAX_ROUNDS: usize = 3;

/// What a check of a row says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seen {
    /// The caller sees the row.
    Yes,
    /// The caller has no right: the route deletes the row.
    Refused,
    /// The check failed: the route hides the row and keeps it.
    Unknown,
}

/// Returns the counts of the inbox of a user.
pub(super) async fn summary(pool: &PgPool, user: ActorId) -> Result<Summary, Error> {
    let mut tx = pool.begin().await?;
    let summary = inbox::summary(&mut tx, user).await?;
    tx.commit().await?;
    Ok(summary)
}

/// Returns one page of the inbox of the caller. It deletes a row that a clear refusal stops. When
/// each row of a page is out of the answer, it reads the next page.
pub(super) async fn list(
    pool: &PgPool,
    gate: &dyn Gate,
    kinds: &NotificationKinds,
    caller: &Principal,
    filter: &Filter,
    page: PageRequest<NotificationId>,
) -> Result<Page<Notification, NotificationId>, Error> {
    let user = caller.actor.id;
    let mut cursor = page;
    let mut items = Vec::new();
    let mut next_after = None;
    for _ in 0..MAX_ROUNDS {
        // Read one page with one extra row.
        let mut tx = pool.begin().await?;
        let mut rows = inbox::list(&mut tx, user, filter, cursor).await?;
        tx.commit().await?;
        let more = rows.len() > cursor.size.get();
        rows.truncate(cursor.size.get());

        // Check each row, and delete the refused ones.
        let seen = check(gate, kinds, caller, &rows).await;
        let refused: Vec<Uuid> = rows
            .iter()
            .zip(&seen)
            .filter(|(_, seen)| **seen == Seen::Refused)
            .map(|(row, _)| row.id.to_db())
            .collect();
        if !refused.is_empty() {
            delete(pool, user, &refused).await?;
        }

        // Keep the rows of the answer, and go on after the last row that stays.
        let last = rows
            .iter()
            .zip(&seen)
            .rfind(|(_, seen)| **seen != Seen::Refused)
            .map(|(row, _)| row.id);
        let found = rows
            .into_iter()
            .zip(seen)
            .filter(|(_, seen)| *seen == Seen::Yes)
            .map(|(row, _)| with_names(row));
        items.extend(found);
        cursor.after = last.or(cursor.after);
        next_after = more.then_some(cursor.after).flatten();
        if !more || !items.is_empty() {
            break;
        }
    }
    Ok(Page { items, next_after })
}

/// Deletes rows of a user.
async fn delete(pool: &PgPool, user: ActorId, ids: &[Uuid]) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    inbox::delete(&mut tx, user, ids).await?;
    tx.commit().await?;
    Ok(())
}

/// Returns the row with the snapshot of its names, if the row is about the reader: the reader can
/// have no access to read the names of today.
fn with_names(mut row: Notification) -> Notification {
    if row.reason == REASON_SUBJECT {
        row.workspace_name = row.params.get(WORKSPACE_NAME).cloned();
        row.project_name = row.params.get(PROJECT_NAME).cloned();
    }
    row
}

/// Returns the object of the check of a row: its project, else its workspace, else the instance.
fn object_of(row: &Notification) -> (Object, Option<WorkspaceId>) {
    let object = match (row.project_id, row.workspace_id) {
        (Some(project), _) => Object::of(project),
        (None, Some(workspace)) => Object::of(workspace),
        (None, None) => Object::of(Platform::ID),
    };
    (object, row.workspace_id)
}

/// Checks each row, with one call of the gate for each pair of an object and a permission. A row
/// of a kind that the edition does not have stays hidden.
async fn check(
    gate: &dyn Gate,
    kinds: &NotificationKinds,
    caller: &Principal,
    rows: &[Notification],
) -> Vec<Seen> {
    let mut known: HashMap<(Object, &'static str), Seen> = HashMap::new();
    let mut seen = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(kind) = kinds.get(&row.kind) else {
            seen.push(Seen::Unknown);
            continue;
        };
        let permission = match kind.access {
            Access::Holds(permission) if row.reason != REASON_SUBJECT => permission,
            _ => {
                seen.push(Seen::Yes);
                continue;
            }
        };
        let (object, workspace) = object_of(row);
        let key = (object.clone(), permission);
        let result = match known.get(&key) {
            Some(result) => *result,
            None => {
                let answer = gate
                    .permissions(caller, &object, workspace, &[permission])
                    .await;
                let result = match answer.as_deref() {
                    Ok([true]) => Seen::Yes,
                    Ok(_) => Seen::Refused,
                    Err(_) => Seen::Unknown,
                };
                known.insert(key, result);
                result
            }
        };
        seen.push(result);
    }
    seen
}

/// Refuses too many ids, and an id that is not a row of the user.
async fn check_owned(tx: &mut Tx<'_>, user: ActorId, ids: &[Uuid]) -> Result<(), Error> {
    if ids.len() > MAX_IDS {
        return Err(Error::invalid(
            "notification_ids",
            format!("has more than {MAX_IDS} ids"),
        ));
    }
    let distinct: HashSet<&Uuid> = ids.iter().collect();
    if inbox::count_owned(tx, user, ids).await? != distinct.len() as i64 {
        return Err(Error::NotFound("notification"));
    }
    Ok(())
}

/// Marks rows of a user as read or as unread.
pub(super) async fn set_read(
    pool: &PgPool,
    user: ActorId,
    ids: &[NotificationId],
    read: bool,
) -> Result<(), Error> {
    let ids: Vec<Uuid> = ids.iter().map(|id| id.to_db()).collect();
    let mut tx = pool.begin().await?;
    check_owned(&mut tx, user, &ids).await?;
    inbox::set_read(&mut tx, user, &ids, read).await?;
    tx.commit().await?;
    Ok(())
}

/// Marks each row of a user as read, except a row that waits for an action.
pub(super) async fn set_all_read(pool: &PgPool, user: ActorId) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    inbox::set_all_read(&mut tx, user).await?;
    tx.commit().await?;
    Ok(())
}

/// Marks rows of a user as done, or puts them back in the inbox.
pub(super) async fn set_done(
    pool: &PgPool,
    user: ActorId,
    ids: &[NotificationId],
    done: bool,
) -> Result<(), Error> {
    let ids: Vec<Uuid> = ids.iter().map(|id| id.to_db()).collect();
    let mut tx = pool.begin().await?;
    check_owned(&mut tx, user, &ids).await?;
    inbox::set_done(&mut tx, user, &ids, done).await?;
    tx.commit().await?;
    Ok(())
}
