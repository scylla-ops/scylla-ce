//! The conversions of the activity feed between the domain and the protos.

use scylla_domain::activity::Activity;
use scylla_proto::activity::v1 as pb;

use crate::api::parse::{self, ToProto};

impl ToProto<pb::Activity> for Activity {
    fn to_proto(self) -> pb::Activity {
        let (subject_kind, subject_id) = match self.subject {
            Some(subject) => (subject.kind, Some(subject.id.to_proto())),
            None => (String::new(), None),
        };
        pb::Activity {
            id: Some(self.id.to_proto()),
            kind: self.kind,
            area: self.area,
            actor_id: self.actor_id.map(ToProto::to_proto),
            actor_name: self.actor_name.unwrap_or_default(),
            project_id: self.project_id.map(ToProto::to_proto),
            project_name: self.project_name.unwrap_or_default(),
            subject_kind,
            subject_id,
            params: self.params.into_iter().collect(),
            at: Some(parse::timestamp(self.at)),
        }
    }
}
