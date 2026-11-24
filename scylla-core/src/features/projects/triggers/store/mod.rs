//! The SQL of the triggers: the `triggers`, `trigger_runners`, `trigger_deliveries` and
//! `trigger_fires` tables, and the triggers of the `on` table of a file pipeline. Each function
//! runs in the transaction of the caller.

pub(crate) mod deliveries;
pub(crate) mod files;
pub(crate) mod fire_figures;
pub(crate) mod fires;
mod row;
pub(crate) mod runners;
pub(crate) mod schedules;
pub(crate) mod summaries;
pub(crate) mod triggers;
