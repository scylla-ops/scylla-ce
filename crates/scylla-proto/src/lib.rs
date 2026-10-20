#![allow(clippy::all, clippy::pedantic, clippy::nursery, clippy::cargo)]

pub mod common {
    pub mod v1 {
        tonic::include_proto!("scylla.common.v1");
    }
}

pub mod exec {
    pub mod v1 {
        tonic::include_proto!("scylla.exec.v1");
    }
}

pub mod auth {
    pub mod v1 {
        tonic::include_proto!("scylla.auth.v1");
    }
}

pub mod user {
    pub mod v1 {
        tonic::include_proto!("scylla.user.v1");
    }
}

pub mod organization {
    pub mod v1 {
        tonic::include_proto!("scylla.organization.v1");
    }
}

pub mod project {
    pub mod v1 {
        tonic::include_proto!("scylla.project.v1");
    }
}

pub mod authz {
    pub mod v1 {
        tonic::include_proto!("scylla.authz.v1");
    }
}

pub mod pipeline {
    pub mod v1 {
        tonic::include_proto!("scylla.pipeline.v1");
    }
}

pub mod secret {
    pub mod v1 {
        tonic::include_proto!("scylla.secret.v1");
    }
}

pub mod job {
    pub mod v1 {
        tonic::include_proto!("scylla.job.v1");
    }
}

pub mod app {
    pub mod v1 {
        tonic::include_proto!("scylla.app.v1");
    }
}

pub mod agent {
    pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
    pub const MAX_LOG_LINE_BYTES: usize = 64 * 1024;

    pub mod v1 {
        tonic::include_proto!("scylla.agent.v1");
    }
}

pub mod trigger {
    pub mod v1 {
        tonic::include_proto!("scylla.trigger.v1");
    }
}

pub mod convert;

pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("scylla_descriptor");
