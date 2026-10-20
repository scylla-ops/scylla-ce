use super::command::{Command, Describe, Query};
use super::envelope::Envelope;
use super::id::ActionId;
use super::phase::{Authorized, Committed, Fetched, Prepared, Requested};
use crate::authz::Access;
use crate::domain::caller::CallerContext;
use chrono::{DateTime, Utc};
use std::any::Any;
use std::ops::Deref;
use std::sync::Arc;

/// The erased view of any phase. An erased hook (`Policy`, `Around`, `Observer`) fires for
/// commands that do not exist yet, so it only gets what every command has; `downcast_ref` reaches
/// the typed phase when the hook knows the command.
pub trait Action: Send + Sync {
    fn id(&self) -> &ActionId;
    fn at(&self) -> DateTime<Utc>;
    fn caller(&self) -> &CallerContext;
    fn access(&self) -> &Access;
    fn as_any(&self) -> &dyn Any;
}

/// A phase can hand out its envelope, which is what an observer sees when a stage failed after
/// consuming the input.
pub trait Phase: Action {
    fn envelope(&self) -> Arc<dyn Action>;
}

impl dyn Action + '_ {
    #[must_use]
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.as_any().downcast_ref()
    }
}

impl<C: Describe> Action for Envelope<C> {
    fn id(&self) -> &ActionId {
        Envelope::id(self)
    }

    fn at(&self) -> DateTime<Utc> {
        Envelope::at(self)
    }

    fn caller(&self) -> &CallerContext {
        Envelope::caller(self)
    }

    fn access(&self) -> &Access {
        Envelope::access(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

macro_rules! phase {
    ($($phase:ident: $bound:ident),*) => {
        $(
            impl<C: $bound> Deref for $phase<C> {
                type Target = Envelope<C>;

                fn deref(&self) -> &Envelope<C> {
                    &self.env
                }
            }

            impl<C: $bound> Action for $phase<C> {
                fn id(&self) -> &ActionId {
                    self.env.id()
                }

                fn at(&self) -> DateTime<Utc> {
                    self.env.at()
                }

                fn caller(&self) -> &CallerContext {
                    self.env.caller()
                }

                fn access(&self) -> &Access {
                    self.env.access()
                }

                fn as_any(&self) -> &dyn Any {
                    self
                }
            }

            impl<C: $bound> Phase for $phase<C> {
                fn envelope(&self) -> Arc<dyn Action> {
                    self.env.clone()
                }
            }
        )*
    };
}

phase!(
    Requested: Describe,
    Authorized: Describe,
    Prepared: Command,
    Committed: Command,
    Fetched: Query
);
