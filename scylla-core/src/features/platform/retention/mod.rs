//! The retention feature: a task deletes the rows with personal data some time after their use,
//! as the settings tell.

mod service;
mod settings;
mod store;

pub use service::run;
pub(crate) use service::spawn;
pub use settings::RetentionSettings;
