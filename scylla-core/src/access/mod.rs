//! Zero-trust access: short-lived Biscuit tokens, revocation, and SpiceDB permissions.

mod commit;
mod gate;
mod keys;
mod operation;
mod spicedb;
mod store;

pub use commit::commit;
pub use gate::{Access, AccessConfig};
pub use keys::TokenKeys;
pub use operation::OperationLayer;
pub use spicedb::SpiceDb;
