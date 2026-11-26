//! The SQL of the `workspaces` and `workspace_owners` tables, and of the icons of the
//! workspaces. Each function runs in the transaction of the caller.

mod icons;
mod limit;
mod listing;
mod owners;
mod row;
mod workspaces;

pub(crate) use icons::{lock_icon, set_icon};

pub(crate) use limit::{count_for, counted_for, has_no_limit, lock_creator};
pub(crate) use listing::{facts, list_all, owners_of, row_roles_of};

pub(crate) use owners::{add_owner, count_owners, leave_all, lock_sole_owned, remove_owner};
pub(crate) use workspaces::{
    delete_all, exists, find, find_by_slug, has_content, insert, list_visible, lock,
    refusing_personal_tokens, update,
};
