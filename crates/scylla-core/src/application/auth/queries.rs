//! The session's reads. Each query, its access and its output type. The strata module of the slice
//! runs it.

use scylla_extension::{Access, Describe, Query};

/// No `Debug`: `token` is the session credential.
pub struct ValidateToken {
    pub token: String,
}

impl Describe for ValidateToken {
    fn access(&self) -> Access {
        Access::Public
    }
}

impl Query for ValidateToken {
    type Output = bool;
}
