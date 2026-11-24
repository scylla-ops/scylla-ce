//! The proto values of the merge blocks: a rule, and its state.

use scylla_domain::integrations::merge_blocks::MergeBlockState;
use scylla_proto::integrations::v1 as pb;

use super::view::Block;
use crate::api::parse::{self, ToProto};

impl ToProto<pb::MergeBlock> for Block {
    fn to_proto(self) -> pb::MergeBlock {
        pb::MergeBlock {
            project_id: Some(self.project.to_proto()),
            pipeline_id: self.pipeline.map(ToProto::to_proto),
            context: self.context.unwrap_or_default(),
            enabled: self.enabled,
            state: state(self.state) as i32,
            state_reason: self.reason.unwrap_or_default(),
            branch: self.branch,
            action_url: self.action_url,
            updated_at: self.updated_at.map(parse::timestamp),
        }
    }
}

/// Returns the proto value of a state.
fn state(value: MergeBlockState) -> pb::MergeBlockState {
    match value {
        MergeBlockState::Off => pb::MergeBlockState::Off,
        MergeBlockState::Applying => pb::MergeBlockState::Applying,
        MergeBlockState::On => pb::MergeBlockState::On,
        MergeBlockState::NeedsPermission => pb::MergeBlockState::NeedsPermission,
        MergeBlockState::NotPossible => pb::MergeBlockState::NotPossible,
        MergeBlockState::OutOfSync => pb::MergeBlockState::OutOfSync,
        MergeBlockState::Failed => pb::MergeBlockState::Failed,
    }
}
