//! The agents: the Apps that run jobs, with their stream.

use crate::identity::actors::Actor;

crate::authz! {
    /// The routes of an agent on itself.
    extend Actor as AgentCaller {
        /// The route `OpenStream`: an agent opens its own stream.
        permission open_stream = nil;
    }
}
