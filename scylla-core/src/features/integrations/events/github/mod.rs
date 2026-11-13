//! The webhooks of GitHub: their payloads, and what each one asks for.

mod payload;
mod read;

pub(crate) use read::read;
