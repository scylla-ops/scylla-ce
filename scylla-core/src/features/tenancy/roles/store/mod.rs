//! The SQL of the custom roles, their permissions and their grants. Each function runs in the
//! transaction of the caller.

mod grants;
mod rows;

pub(crate) use grants::*;
pub(crate) use rows::*;
