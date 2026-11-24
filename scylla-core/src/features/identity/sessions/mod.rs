//! The sessions feature: one session for each sign-in of a user, with a refresh token that
//! changes at each use, the client of the sign-in, and the cookies of a browser.

mod client;
pub mod cookie;
mod service;
mod store;

pub use client::Client;
pub use service::Opened;
pub(crate) use service::{close, close_one, of_user, open, refresh, user_of};
pub(crate) use store::revoke_all;
pub use store::SessionInfo;
