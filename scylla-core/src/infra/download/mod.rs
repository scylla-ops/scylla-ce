//! The download of a file of another server for a person, as the photo that an identity provider
//! gives: HTTPS to a public address only, no redirect, no credential, a short timeout, and a size
//! limit while the body comes.

mod address;
mod fetch;

#[cfg(test)]
mod tests;

pub(crate) use fetch::{download, Reach};
