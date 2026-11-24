//! The SQL of the access: the owners, the profiles, the grants that are rows and the time of the
//! grants of the schema.

mod access;
mod rows;

pub(crate) use access::*;
pub(crate) use rows::*;
