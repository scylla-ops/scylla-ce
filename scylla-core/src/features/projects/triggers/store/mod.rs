//! The SQL of the triggers: the `triggers`, `trigger_runners` and `trigger_deliveries` tables,
//! and the triggers of the `on` table of a file pipeline. Each function runs in the transaction of
//! the caller.

pub(crate) mod deliveries;
pub(crate) mod files;
pub(crate) mod runners;
pub(crate) mod triggers;
