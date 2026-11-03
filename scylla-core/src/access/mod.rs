//! Zero-trust access: short-lived Biscuit tokens, revocation, ownership, and SpiceDB grants.

mod claims;
mod gate;
mod keys;
mod operation;
mod relationships;
mod rights;
mod spicedb;
mod store;

pub use gate::{Access, AccessConfig};
pub use keys::TokenKeys;
pub use operation::OperationLayer;
pub use spicedb::SpiceDb;
