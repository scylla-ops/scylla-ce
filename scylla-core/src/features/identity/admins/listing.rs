//! The list of the console of the users: each user with its instance roles, its workspaces and its
//! last sign-in.

use std::collections::HashMap;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::admins::{InstanceRole, ListUsers, UserEntries, UserEntry};
use scylla_domain::tenancy::roles::{builtin, INSTANCE_ADMIN};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_domain::Page;

use super::store;
use crate::features::identity::accounts::{self, AccountContext};
use crate::Error;

/// Returns the workspaces where each user has a role. A failed lookup gives none for that user.
async fn granted_workspaces(
    ctx: &AccountContext,
    users: &[ActorId],
) -> HashMap<ActorId, Vec<WorkspaceId>> {
    let mut granted = HashMap::new();
    for &user in users {
        let ids = ctx
            .gate
            .granted_to(user, Workspace::LIST_WORKSPACES)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "a lookup failed: the list shows only the owned workspaces");
                Vec::new()
            });
        granted.insert(user, ids);
    }
    granted
}

/// Returns one page of the console of the users.
pub(super) async fn list(ctx: &AccountContext, query: ListUsers) -> Result<UserEntries, Error> {
    // Read the page, and the workspaces of its users in the permission system.
    let text = query.query.clone();
    let page = accounts::list_users(&ctx.pool, query).await?;
    let users: Vec<ActorId> = page.items.iter().map(|user| user.id).collect();
    let granted = granted_workspaces(ctx, &users).await;
    let granted_ids: Vec<WorkspaceId> = granted.values().flatten().copied().collect();

    // Read the facts of the users of the page.
    let mut tx = ctx.pool.begin().await?;
    let roles = store::platform_roles(&mut tx, &users).await?;
    let owned = store::owned_workspaces(&mut tx, &users).await?;
    let names = store::workspace_names(&mut tx, &granted_ids).await?;
    let sign_ins = store::last_sign_ins(&mut tx, &users).await?;
    let (total, total_with_instance_role) = store::totals(&mut tx, text.trim()).await?;
    tx.commit().await?;

    // Put the facts on each user.
    let items = page
        .items
        .into_iter()
        .map(|user| {
            let mut instance_roles = Vec::new();
            if user.instance_admin {
                instance_roles.extend(builtin(INSTANCE_ADMIN).map(|role| InstanceRole {
                    key: role.key.to_owned(),
                    name: role.name.to_owned(),
                    builtin: true,
                }));
            }
            instance_roles.extend(
                roles
                    .iter()
                    .filter(|(holder, _)| *holder == user.id)
                    .map(|(_, role)| role.clone()),
            );
            let mut workspaces: HashMap<WorkspaceId, String> = owned
                .iter()
                .filter(|(owner, _, _)| *owner == user.id)
                .map(|(_, id, name)| (*id, name.clone()))
                .collect();
            for id in granted.get(&user.id).into_iter().flatten() {
                if let Some(name) = names.get(id) {
                    workspaces.insert(*id, name.clone());
                }
            }
            let mut workspaces: Vec<String> = workspaces.into_values().collect();
            workspaces.sort();
            UserEntry {
                last_sign_in_at: sign_ins.get(&user.id).copied(),
                user,
                instance_roles,
                workspaces,
            }
        })
        .collect();
    Ok(UserEntries {
        page: Page {
            items,
            next_after: page.next_after,
        },
        total,
        total_with_instance_role,
    })
}
