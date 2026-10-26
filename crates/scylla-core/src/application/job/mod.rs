pub mod log;
pub mod repository;

pub use scylla_domain::JobEvent;

pub use log::{JobLogLiveStream, JobLogRepository, JobLogStreamPort};
pub use repository::{JobRepository, JobScope};

use chrono::TimeDelta;

/// How long a running job stays with an agent that is not connected, after its last contact.
pub const ORPHAN_GRACE: TimeDelta = TimeDelta::seconds(60);
