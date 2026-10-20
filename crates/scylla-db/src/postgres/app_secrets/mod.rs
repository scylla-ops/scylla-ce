pub mod repository;

pub use repository::{PgAppCredentialRepository, insert};

#[cfg(test)]
mod tests;
