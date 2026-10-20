use crate::domain::ids::{AppId, UserId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerContext {
    User(UserId),
    App(AppId),
    Service(ServiceIdentity),
    Anonymous,
}

/// Sealed: only the factories below build one, so a handler cannot forge a service caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceIdentity {
    name: &'static str,
}

impl ServiceIdentity {
    #[must_use]
    pub fn recorder() -> Self {
        Self { name: "recorder" }
    }

    #[must_use]
    pub fn bootstrap() -> Self {
        Self { name: "bootstrap" }
    }

    #[must_use]
    pub fn job_reaper() -> Self {
        Self { name: "job-reaper" }
    }

    #[must_use]
    pub fn job_dispatcher() -> Self {
        Self {
            name: "job-dispatcher",
        }
    }

    #[must_use]
    pub fn cron_scheduler() -> Self {
        Self {
            name: "cron-scheduler",
        }
    }

    #[must_use]
    pub fn trigger_firer() -> Self {
        Self {
            name: "trigger-firer",
        }
    }

    #[must_use]
    pub fn session_sweeper() -> Self {
        Self {
            name: "session-sweeper",
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &'static str {
        self.name
    }
}

impl std::fmt::Display for CallerContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User(id) => write!(f, "user:{}", id.as_str()),
            Self::App(id) => write!(f, "app:{}", id.as_str()),
            Self::Service(svc) => write!(f, "service:{}", svc.as_str()),
            Self::Anonymous => write!(f, "anonymous"),
        }
    }
}
