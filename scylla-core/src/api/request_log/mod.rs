//! The log of each call: its request id, its client IP address, its actor, and one line with
//! its result.

mod body;
mod layer;
mod result;

pub use layer::{RequestLog, REQUEST_ID};
