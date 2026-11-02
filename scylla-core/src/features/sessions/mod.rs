//! The sessions feature: one session for each sign-in of a user, with a refresh token that
//! changes at each use, and the cookies of a browser.

pub mod cookie;
mod service;
mod store;

pub use service::Opened;
pub(crate) use service::{close, open, refresh, user_of};
pub use store::SessionInfo;
pub(crate) use store::{list, revoke, revoke_all};
