pub mod dispatch;
pub mod dispatch_port;
pub mod dispatch_use_case;
pub mod repository;

pub use dispatch::{DispatchEnv, DispatchNode, JobDispatch, MAX_DISPATCH_BYTES};
pub use dispatch_port::{AgentDispatch, AgentOrder, AgentStream};
pub use dispatch_use_case::DispatchUseCases;
pub use repository::{AgentRepository, AgentStats};

use crate::domain::agent::AgentHost;
use crate::domain::app::{App, AppSecret};
use chrono::{DateTime, Utc};

/// The outcome of `create_agent`. No `Debug`: `secret` is the plaintext that the caller sees once.
pub struct CreatedAgent {
    pub app: App,
    pub secret: AppSecret,
}

/// An agent as a read shows it. `connected` comes from the registry, `in_flight` from the jobs
/// placed on the agent.
pub struct AgentView {
    pub app: App,
    pub connected: bool,
    pub last_seen: Option<DateTime<Utc>>,
    pub in_flight: u32,
    pub host: Option<AgentHost>,
}
