//! The SQL of the `workspaces` and `workspace_owners` tables. Each function runs in the
//! transaction of the caller.

mod limit;
mod owners;
mod workspaces;

pub(crate) use limit::{count_for, counted_for, has_no_limit, lock_creator};

pub(crate) use owners::{
    add_owner, count_owners, leave_all, lock_sole_owned, owners, remove_owner,
};
pub(crate) use workspaces::{
    delete_all, exists, find, find_by_slug, has_content, insert, list_visible, lock,
    refusing_personal_tokens, update,
};
