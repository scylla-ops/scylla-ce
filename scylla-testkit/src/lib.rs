//! The test kit: the real server of an edition, with a real gate, a mailbox, a capture of the
//! log lines, a fake Git provider, and helpers for the core RPCs, grouped by subject.

mod app;
mod identity;
mod integrations;
mod logs;
mod mail;
mod notifications;
mod pipeline_layouts;
mod pipelines;
mod platform;
mod projects;
mod rpc;
mod tenancy;
mod triggers;

pub use app::{test_access, TestApp};
pub use identity::{new_key, TestKey};
pub use integrations::{
    test_github_app, wait_for_requests, DeliveryRow, FakeProvider, LinkRow, ModeRow, Recorded,
    TestAgent, TEST_APP_CLIENT_SECRET, TEST_APP_PRIVATE_KEY, TEST_APP_PUBLIC_KEY,
    TEST_APP_WEBHOOK_SECRET,
};
pub use logs::Logs;
pub use mail::Mailbox;
pub use pipelines::exec_node;
pub use rpc::{id, text, with_token};
pub use tenancy::{Role, RoleInput};
pub use triggers::{literal_input, pointer_input};
