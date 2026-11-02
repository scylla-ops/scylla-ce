//! The generated code of `proto/`. Docs come from the proto comments, which `buf lint` checks.

/// The capabilities API.
pub mod capabilities {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.capabilities.v1");
    }
}

/// The shared messages: the id and the page.
pub mod common {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.common.v1");
    }
}

/// The projects API.
pub mod projects {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.projects.v1");
    }
}

/// The agents API.
pub mod agents {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.agents.v1");
    }
}

/// The token API.
pub mod auth {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.auth.v1");
    }
}
