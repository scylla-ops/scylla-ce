//! The SQL of the reports: the outbox writes of a change of a job, the queue of the sender, the
//! reads of the routes, and the people that a notice reaches.

pub(crate) mod outbox;
pub(crate) mod people;
pub(crate) mod queue;
pub(crate) mod reads;
