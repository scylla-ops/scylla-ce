//! The SQL of the reports: the outbox writes of a change of a job, the queue of the sender, the
//! reads of the routes, the people that a notice reaches, and the reads of the check runs.

pub(crate) mod checks;
pub(crate) mod outbox;
pub(crate) mod people;
pub(crate) mod queue;
pub(crate) mod reads;
