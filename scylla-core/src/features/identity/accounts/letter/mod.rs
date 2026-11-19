//! The layout of the account mails: a text body and an HTML body from the same content, in the
//! look of the web app, with the logo as the only image.

mod content;
mod html;
mod style;
#[cfg(test)]
mod tests;
mod text;

pub(crate) use content::{clock, Action, Badge, Fact, Letter, NotYou, Part};
