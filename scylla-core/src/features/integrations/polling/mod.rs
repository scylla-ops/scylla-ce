//! The polling of the linked repositories: a task reads the heads of the branches and of the open
//! pull requests at the interval of each link in polling mode, and each 15 minutes in webhook
//! mode to find a lost delivery.

mod deliver;
mod github;
mod gitlab;
mod listed;
mod mount;
mod poll;
pub(crate) mod store;
mod task;

pub(crate) use mount::mount;
pub(crate) use poll::{to_polling, APP_HAS_NO_WEBHOOK, SAFETY_SECONDS, WEBHOOKS_NOT_ARRIVING};
pub(crate) use store::heads;
