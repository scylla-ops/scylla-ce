//! The adapters of the core: the database, the logs, the mails, the secrets, the guard against
//! the public dev keys, the TLS crypto provider, the rate limits in memory, the download of a file
//! of another server, and the clients of the Git providers.

pub mod db;
pub mod dev_keys;
pub(crate) mod download;
pub mod logs;
pub mod mail;
pub(crate) mod rates;
pub mod secret;
pub mod tls;
pub mod vcs;
