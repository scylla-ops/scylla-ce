//! The logs of the server and of the commands: one line for each call and for each event, pretty
//! for a person or JSON for a machine, with no secret and no full email.

mod format;
mod json;
mod layer;
mod mask;
mod pretty;
mod record;
mod setup;
#[cfg(test)]
mod tests;

pub use format::{LogFormat, Style};
pub use layer::{annotate, LogLayer, CALL};
pub use mask::{mask_email, mask_identifier};
pub use record::Outcome;
pub use setup::{capture, init, DEFAULT_FILTER};
