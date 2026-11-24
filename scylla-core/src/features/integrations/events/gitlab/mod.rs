//! The webhooks of GitLab: their payloads, and the Git event of each one.

mod payload;
mod read;

pub(crate) use read::read;
