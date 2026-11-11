//! The conversions of the pipeline files between the domain and the protos.

use scylla_domain::projects::git_rules::{GitRules, PullRequestAction};
use scylla_domain::projects::pipeline_files::{CheckoutSettings, FileError, FileFormat};
use scylla_proto::pipelines::v1 as pb;

use super::files_service::CheckedFile;
use crate::api::parse::ToProto;
use crate::Error;

/// Reads the format of a request. No format gives `INVALID_ARGUMENT`.
pub(super) fn format(raw: i32) -> Result<FileFormat, Error> {
    match pb::PipelineFileFormat::try_from(raw) {
        Ok(pb::PipelineFileFormat::Toml) => Ok(FileFormat::Toml),
        Ok(pb::PipelineFileFormat::Yaml) => Ok(FileFormat::Yaml),
        Ok(pb::PipelineFileFormat::Json) => Ok(FileFormat::Json),
        Ok(pb::PipelineFileFormat::Unspecified) | Err(_) => {
            Err(Error::invalid("format", "is required"))
        }
    }
}

/// Returns the proto of a format.
fn format_of(format: FileFormat) -> pb::PipelineFileFormat {
    match format {
        FileFormat::Toml => pb::PipelineFileFormat::Toml,
        FileFormat::Yaml => pb::PipelineFileFormat::Yaml,
        FileFormat::Json => pb::PipelineFileFormat::Json,
    }
}

/// Returns the proto of an error, or of a warning.
fn error_of(error: FileError, warning: bool) -> pb::FileError {
    let location = error.location;
    pb::FileError {
        path: error.path,
        line: location.map_or(0, |place| place.line),
        column: location.map_or(0, |place| place.column),
        field: error.field,
        message: error.message,
        warning,
    }
}

/// Returns the proto of the checkout settings.
pub(super) fn checkout_of(checkout: CheckoutSettings) -> pb::CheckoutSettings {
    pb::CheckoutSettings {
        enabled: checkout.enabled,
        depth: checkout.depth,
        submodules: checkout.submodules,
        lfs: checkout.lfs,
    }
}

/// Returns the proto of an action on a pull request.
fn event_of(action: PullRequestAction) -> pb::PullRequestEvent {
    match action {
        PullRequestAction::Opened => pb::PullRequestEvent::Opened,
        PullRequestAction::NewCommits => pb::PullRequestEvent::Synchronized,
        PullRequestAction::Reopened => pb::PullRequestEvent::Reopened,
        PullRequestAction::ReadyForReview => pb::PullRequestEvent::ReadyForReview,
    }
}

/// Puts the rules of the Git events in the proto of a file.
fn put_rules(rules: GitRules, file: &mut pb::PipelineFile) {
    file.push = rules.push.map(|rule| pb::Push {
        branches: rule.branches,
        exclude_branches: rule.exclude,
        skip_ci: rule.skip_ci,
    });
    file.pull_request = rules.pull_request.map(|rule| pb::PullRequest {
        branches: rule.branches,
        events: rule
            .events
            .into_iter()
            .map(|action| event_of(action).into())
            .collect(),
        drafts: rule.drafts,
    });
    file.tag = rules.tag.map(|rule| pb::Tag {
        patterns: rule.patterns,
    });
}

/// Returns the proto of a file that a client sends, with its text.
pub(super) fn checked_file(checked: CheckedFile, content: String) -> pb::PipelineFile {
    // Write what the file is.
    let CheckedFile {
        file,
        pipeline,
        warnings,
        pipeline_id,
    } = checked;
    let mut proto = pb::PipelineFile {
        key: file.key.as_str().to_owned(),
        file: Some(pb::PipelineFileRef {
            path: file.path,
            ..pb::PipelineFileRef::default()
        }),
        format: format_of(file.format).into(),
        content,
        name: file.key.as_str().to_owned(),
        pipeline_id: pipeline_id.map(ToProto::to_proto),
        ..pb::PipelineFile::default()
    };

    // Write its pipeline and its warnings, or its errors.
    match pipeline {
        Ok(pipeline) => {
            proto.name = pipeline.name.as_str().to_owned();
            proto.nodes = pipeline
                .nodes
                .as_slice()
                .iter()
                .cloned()
                .map(ToProto::to_proto)
                .collect();
            proto.checkout = Some(checkout_of(pipeline.checkout));
            put_rules(pipeline.rules, &mut proto);
            proto.errors = warnings.into_iter().map(|w| error_of(w, true)).collect();
        }
        Err(errors) => {
            proto.errors = errors.into_iter().map(|e| error_of(e, false)).collect();
        }
    }
    proto
}
