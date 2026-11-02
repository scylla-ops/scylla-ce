//! The transaction that the stores and the hooks share.

/// A Postgres transaction. A hook runs in the transaction of the core write.
pub type Tx<'c> = sqlx::Transaction<'c, sqlx::Postgres>;
