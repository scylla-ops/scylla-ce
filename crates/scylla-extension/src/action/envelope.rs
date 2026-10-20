use super::command::Describe;
use super::id::ActionId;
use crate::authz::Access;
use crate::domain::caller::CallerContext;
use crate::domain::clock;
use chrono::{DateTime, Utc};

/// Built once per `Actions::run` and shared by every phase. The access rule is computed here,
/// once, so a hook reads it without calling the command again.
pub struct Envelope<C> {
    id: ActionId,
    at: DateTime<Utc>,
    caller: CallerContext,
    access: Access,
    command: C,
}

impl<C: Describe> Envelope<C> {
    pub(super) fn new(caller: CallerContext, command: C) -> Self {
        Self {
            id: ActionId::generate(),
            at: clock::now(),
            access: command.access(),
            caller,
            command,
        }
    }
}

impl<C> Envelope<C> {
    pub fn id(&self) -> &ActionId {
        &self.id
    }

    pub fn at(&self) -> DateTime<Utc> {
        self.at
    }

    pub fn caller(&self) -> &CallerContext {
        &self.caller
    }

    pub fn access(&self) -> &Access {
        &self.access
    }

    pub fn command(&self) -> &C {
        &self.command
    }
}
