//! The agents: the processes that run the steps of a pipeline for a user.

use crate::actors::{Actor, ActorKind};
use crate::Id;

/// The actor kind of an agent.
pub const KIND: ActorKind = ActorKind::new("agent");

/// The marker of an agent key id.
pub enum AgentKey {}

/// The id of a public key of an agent.
pub type AgentKeyId = Id<AgentKey>;

crate::authz! {
    /// The routes of the agents that act on their owner.
    extend Actor as AgentOwner {
        /// The route `RegisterAgent`, for this owner.
        permission register_agent = act_for;
    }
}
