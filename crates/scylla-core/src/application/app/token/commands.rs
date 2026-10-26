//! The app token exchange's writes. Each command, its access and its payload types. The strata
//! module of the slice runs it.

use crate::domain::app::{AppSecret, AppToken};
use crate::domain::ids::AppId;
use scylla_extension::{Access, Command, Describe, Draft};

#[derive(Debug)]
pub struct IssueAppToken {
    pub app_id: AppId,
    pub secret: AppSecret,
}

impl Describe for IssueAppToken {
    fn access(&self) -> Access {
        Access::Public
    }
}

impl Command for IssueAppToken {
    type Staged = Draft<AppToken>;
    type Committed = AppToken;
}
