//! The SQL of the triggers: the `triggers`, `trigger_runners` and `trigger_deliveries` tables.
//! Each function runs in the transaction of the caller.

pub(crate) mod deliveries;
pub(crate) mod runners;
pub(crate) mod triggers;
