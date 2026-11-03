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

/// The projects API: the projects and their secrets.
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

/// The access API: the grants of roles.
pub mod access {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.access.v1");
    }
}

/// The workspaces API.
pub mod workspaces {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.workspaces.v1");
    }
}

/// The accounts API: sign-up, sign-in with a password, the profile of the caller, and the
/// console of the users.
pub mod accounts {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.accounts.v1");
    }
}
