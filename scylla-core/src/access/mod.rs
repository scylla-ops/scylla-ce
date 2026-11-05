//! Zero-trust access: short-lived Biscuit tokens, revocation, ownership, and SpiceDB grants.

mod claims;
mod gate;
mod keys;
mod limits;
mod operation;
mod refusals;
mod relationships;
mod rights;
mod setup;
mod spicedb;
mod store;

pub use gate::{Access, AccessConfig};
pub use keys::TokenKeys;
pub use limits::attenuate;
pub use operation::OperationLayer;
pub use spicedb::SpiceDb;
