use crate::domain::clock;
use crate::domain::ids::{JobId, JobLogId};
use crate::domain::job::LogStream;
use crate::domain::pipeline::NodeId;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct JobLog {
    id: JobLogId,
    job_id: JobId,
    node_id: NodeId,
    stream: LogStream,
    line: String,
    timestamp: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

impl JobLog {
    #[must_use]
    pub fn from_persistence(
        id: JobLogId,
        job_id: JobId,
        node_id: NodeId,
        stream: LogStream,
        line: String,
        timestamp: DateTime<Utc>,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            job_id,
            node_id,
            stream,
            line,
            timestamp,
            created_at,
        }
    }

    #[must_use]
    pub fn new(
        job_id: JobId,
        node_id: NodeId,
        stream: LogStream,
        line: String,
        timestamp: DateTime<Utc>,
    ) -> Self {
        Self {
            id: JobLogId::generate(),
            job_id,
            node_id,
            stream,
            line: if line.contains('\0') {
                line.replace('\0', "\u{FFFD}")
            } else {
                line
            },
            timestamp,
            created_at: clock::now(),
        }
    }

    #[must_use]
    pub fn id(&self) -> &JobLogId {
        &self.id
    }

    #[must_use]
    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }

    #[must_use]
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    #[must_use]
    pub fn stream(&self) -> &LogStream {
        &self.stream
    }

    #[must_use]
    pub fn line(&self) -> &str {
        &self.line
    }

    #[must_use]
    pub fn timestamp(&self) -> DateTime<Utc> {
        self.timestamp
    }

    #[must_use]
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nul_character_is_stored_as_the_replacement_character() {
        let log = JobLog::new(
            JobId::generate(),
            NodeId::new("n1").unwrap(),
            LogStream::Stdout,
            "x\0y".into(),
            clock::now(),
        );
        assert_eq!(log.line(), "x\u{FFFD}y");
    }
}
