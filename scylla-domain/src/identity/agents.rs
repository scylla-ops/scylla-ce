//! The agents: the processes of a workspace that run the steps of a pipeline.

use crate::identity::actors::ActorKind;
use crate::tenancy::workspaces::Workspace;
use crate::Id;

/// The actor kind of an agent.
pub const KIND: ActorKind = ActorKind::new("agent");

/// The marker of an agent key id.
pub enum AgentKey {}

/// The id of a public key of an agent.
pub type AgentKeyId = Id<AgentKey>;

crate::authz! {
    /// The routes of the agents on their workspace.
    extend Workspace as AgentHolder {
        /// The route `RegisterAgent`, in this workspace: its admins.
        permission register_agent = manage;
    }
}
