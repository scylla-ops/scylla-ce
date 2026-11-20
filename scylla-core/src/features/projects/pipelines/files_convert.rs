//! The conversions of the pipeline files between the domain and the protos.

use scylla_domain::integrations::links::PipelineSource;
use scylla_domain::projects::git_rules::{
    GitRules, PullRequestAction, PullRequestRule, PushRule, TagRule,
};
use scylla_domain::projects::pipeline_files::{
    CheckoutSettings, FileError, FileFormat, FilePipeline, Rejected,
};
use scylla_proto::integrations::v1 as pb_integrations;
use scylla_proto::pipelines::v1 as pb;

use super::files_service::{CheckedFile, FileAt};
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

/// Reads the source of a request. No source gives `INVALID_ARGUMENT`.
pub(super) fn source(raw: i32) -> Result<PipelineSource, Error> {
    match pb_integrations::PipelineSource::try_from(raw) {
        Ok(pb_integrations::PipelineSource::Editor) => Ok(PipelineSource::Editor),
        Ok(pb_integrations::PipelineSource::File) => Ok(PipelineSource::File),
        Ok(pb_integrations::PipelineSource::Unspecified) | Err(_) => {
            Err(Error::invalid("source", "is required"))
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

/// Returns the proto of the rule of a push.
pub(crate) fn push_of(rule: PushRule) -> pb::Push {
    pb::Push {
        branches: rule.branches,
        exclude_branches: rule.exclude,
        skip_ci: rule.skip_ci,
    }
}

/// Returns the proto of the rule of a pull request.
pub(crate) fn pull_request_of(rule: PullRequestRule) -> pb::PullRequest {
    pb::PullRequest {
        branches: rule.branches,
        events: rule
            .events
            .into_iter()
            .map(|action| event_of(action).into())
            .collect(),
        drafts: rule.drafts,
    }
}

/// Returns the proto of the rule of a tag.
pub(crate) fn tag_of(rule: TagRule) -> pb::Tag {
    pb::Tag {
        patterns: rule.patterns,
    }
}

/// Puts the rules of the Git events in the proto of a file.
fn put_rules(rules: GitRules, file: &mut pb::PipelineFile) {
    file.push = rules.push.map(push_of);
    file.pull_request = rules.pull_request.map(pull_request_of);
    file.tag = rules.tag.map(tag_of);
}

/// Returns the proto of a file that a client sends, with its text.
pub(super) fn checked_file(checked: CheckedFile, content: String) -> pb::PipelineFile {
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
    put_pipeline(&mut proto, pipeline, warnings);
    proto
}

/// Returns the proto of a file of the repository at a commit, with its text if `with_text`.
pub(super) fn file_at(found: FileAt, with_text: bool) -> pb::PipelineFile {
    let mut proto = pb::PipelineFile {
        key: found.key.clone(),
        file: Some(pb::PipelineFileRef {
            path: found.path,
            blob_sha: found.blob_sha,
            commit_sha: found.commit_sha,
            url: found.url,
        }),
        format: found
            .format
            .map_or(pb::PipelineFileFormat::Unspecified, format_of)
            .into(),
        content: if with_text { found.text } else { String::new() },
        name: found.key,
        pipeline_id: found.pipeline_id.map(ToProto::to_proto),
        ..pb::PipelineFile::default()
    };
    put_pipeline(&mut proto, found.pipeline, found.warnings);
    proto
}

/// Returns the proto of the error of a file that the lookup refuses.
pub(super) fn rejected_of(rejected: Rejected) -> pb::FileError {
    error_of(rejected.error, false)
}

/// Puts a pipeline and its warnings, or its errors, in the proto of a file.
fn put_pipeline(
    proto: &mut pb::PipelineFile,
    pipeline: Result<FilePipeline, Vec<FileError>>,
    warnings: Vec<FileError>,
) {
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
            put_rules(pipeline.rules, proto);
            proto.errors = warnings.into_iter().map(|w| error_of(w, true)).collect();
        }
        Err(errors) => {
            proto.errors = errors.into_iter().map(|e| error_of(e, false)).collect();
        }
    }
}
