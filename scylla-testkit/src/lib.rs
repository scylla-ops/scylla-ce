//! The test kit: the real server of an edition, with a real gate, a mailbox, a capture of the
//! log lines, a fake Git provider, and helpers for the core RPCs, grouped by subject.

mod activity;
mod app;
mod identity;
mod integrations;
mod job_stats;
mod logs;
mod mail;
mod nodes;
mod notifications;
mod pipeline_layouts;
mod pipeline_summaries;
mod pipelines;
mod platform;
mod projects;
mod rpc;
mod tenancy;
mod trigger_lists;
mod triggers;

pub use app::{test_access, TestApp};
pub use identity::{new_key, TestKey};
pub use integrations::{
    test_github_app, wait_for_requests, DeliveryRow, FakeProvider, LinkRow, ModeRow, Recorded,
    RunAgent, TestAgent, MANIFEST_CLIENT_SECRET, MANIFEST_WEBHOOK_SECRET, TEST_APP_CLIENT_SECRET,
    TEST_APP_PRIVATE_KEY, TEST_APP_PUBLIC_KEY, TEST_APP_WEBHOOK_SECRET,
};
pub use logs::Logs;
pub use mail::Mailbox;
pub use nodes::{case, exec_node, literal, merge_node, output, rule, switch_node, var};
pub use rpc::{id, text, with_token};
pub use tenancy::{
    dashboard_widgets, env_node, fragment_link, fragment_request, literal_parameter, Role,
    RoleInput,
};
pub use triggers::{header_condition, literal_input, pointer_condition, pointer_input};
