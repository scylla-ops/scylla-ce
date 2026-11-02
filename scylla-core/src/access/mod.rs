//! Zero-trust access: short-lived Biscuit tokens, revocation, and SpiceDB permissions.

mod commit;
mod gate;
mod keys;
mod schema;
mod spicedb;
mod store;

pub use commit::commit;
pub use gate::{Access, AccessConfig};
pub use keys::TokenKeys;
pub use schema::CORE_SCHEMA;
pub use spicedb::SpiceDb;
