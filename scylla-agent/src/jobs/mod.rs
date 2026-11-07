//! The jobs of the agent: the set of the running jobs, the order of their nodes, the attempts of
//! each node, the switches, and their output.

mod command;
mod dag;
mod events;
mod output;
mod process;
mod set;
mod step;
mod switch;

pub(crate) use events::Outbox;
pub(crate) use set::JobSet;
