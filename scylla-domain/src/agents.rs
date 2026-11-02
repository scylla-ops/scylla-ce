//! The agents: the processes that run the steps of a pipeline for a user.

use crate::actors::ActorKind;
use crate::Id;

/// The actor kind of an agent.
pub const KIND: ActorKind = ActorKind::new("agent");

/// The marker of an agent key id.
pub enum AgentKey {}

/// The id of a public key of an agent.
pub type AgentKeyId = Id<AgentKey>;
