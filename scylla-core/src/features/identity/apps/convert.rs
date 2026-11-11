//! The conversions between the proto messages and the domain types of the Apps.

use scylla_domain::identity::apps::{App, AppKeyInfo, Host, KeyLabel};
use scylla_proto::apps::v1 as pb;

use crate::api::parse::{self, FromProto, Invalid, ToProto};

/// Makes the message of an App, with its presence on this server and the number of jobs that it
/// holds.
pub(super) fn app(app: App, connected: bool, running_jobs: u32) -> pb::App {
    pb::App {
        id: Some(app.id.to_proto()),
        workspace_id: Some(app.workspace_id.to_proto()),
        name: app.name,
        runner: app.runner,
        created_by: Some(app.created_by.to_proto()),
        created_at: Some(parse::timestamp(app.created_at)),
        connected,
        last_seen_at: app.last_seen_at.map(parse::timestamp),
        host: app.host.map(host),
        disabled_at: app.disabled_at.map(parse::timestamp),
        running_jobs,
    }
}

/// Reads the optional name of a key: an empty text gives none.
pub(crate) fn optional_label(text: String) -> Result<Option<KeyLabel>, crate::Error> {
    match text.trim().is_empty() {
        true => Ok(None),
        false => parse::field(text, "label").map(Some),
    }
}

/// Makes the message of a host.
fn host(host: Host) -> pb::Host {
    pb::Host {
        version: host.version,
        os: host.os,
        arch: host.arch,
        hostname: host.hostname,
        cpu_count: host.cpu_count,
        memory_bytes: host.memory_bytes,
        capacity: host.capacity,
        container_runtime: host.container_runtime.unwrap_or_default(),
        features: Vec::new(),
    }
}

impl ToProto<pb::AppKey> for AppKeyInfo {
    fn to_proto(self) -> pb::AppKey {
        pb::AppKey {
            id: Some(self.id.to_proto()),
            public_key: self.public_key,
            created_at: Some(parse::timestamp(self.created_at)),
            revoked_at: self.revoked_at.map(parse::timestamp),
            label: self.label.unwrap_or_default(),
            last_used_at: self.last_used_at.map(parse::timestamp),
        }
    }
}

impl FromProto<pb::Host> for Host {
    fn from_proto(value: pb::Host) -> Result<Self, Invalid> {
        let host = Host {
            version: value.version,
            os: value.os,
            arch: value.arch,
            hostname: value.hostname,
            cpu_count: value.cpu_count,
            memory_bytes: value.memory_bytes,
            capacity: value.capacity,
            container_runtime: Some(value.container_runtime),
        };
        Host::parse(host).map_err(|(field, error)| Invalid::at(field, error))
    }
}
