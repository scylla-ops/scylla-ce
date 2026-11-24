//! The agent as a whole: its settings, its errors, and the loop of its streams.

mod error;
mod settings;
mod supervisor;

pub use error::AgentError;
pub use settings::Settings;
pub use supervisor::run;
