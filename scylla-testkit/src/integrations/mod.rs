//! The helpers of the tests of the Git integration: a fake provider, the test GitHub App, the
//! rows of the fixtures, and one module of `TestApp` helpers for each part of the integration.

mod approvals;
mod apps;
mod checkout;
mod checks;
mod connections;
mod events;
mod fakes;
mod files;
mod fixtures;
mod git_triggers;
mod keys;
mod links;
mod merge_blocks;
mod pipeline_files;
mod polling;
mod reports;
mod sources;

pub use checkout::TestAgent;
pub use checks::RunAgent;
pub use events::{DeliveryRow, ModeRow};
pub use fakes::{FakeProvider, Recorded};
pub use fixtures::LinkRow;
pub use keys::{
    test_github_app, TEST_APP_CLIENT_SECRET, TEST_APP_PRIVATE_KEY, TEST_APP_PUBLIC_KEY,
    TEST_APP_WEBHOOK_SECRET,
};
pub use reports::wait_for_requests;
