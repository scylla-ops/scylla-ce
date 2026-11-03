//! The adapters of the core: the database, the logs, the mails, the secrets, the guard against
//! the public dev keys, and the TLS crypto provider.

pub mod db;
pub mod dev_keys;
pub mod logs;
pub mod mail;
pub mod secret;
pub mod tls;
