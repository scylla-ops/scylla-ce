//! The logic of the pipeline files: the check of a file that a client sends, the export of a
//! pipeline as a file, and the read of the files of the linked repository at a commit.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::integrations::links::PipelineSource;
use scylla_domain::integrations::reports::pipeline_key;
use scylla_domain::integrations::sources::GitRef;
use scylla_domain::projects::pipeline_files::{
    self, FileError, FileFormat, FilePipeline, FileRef, PipelineFile, Rejected, TreeEntry,
    MAX_BYTES,
};
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::ProjectId;
use scylla_extension::Rejection;
use sqlx::PgPool;

use super::{service, store};
use crate::features::integrations::{files, IntegrationContext};
use crate::features::projects::{secrets, triggers};
use crate::Error;

/// The folder of the exported files.
const FOLDER: &str = ".scylla/pipelines";

/// A pipeline file that a client sends, as the server reads it.
pub(crate) struct CheckedFile {
    /// The key, the path and the format of the file.
    pub(crate) file: FileRef,
    /// The pipeline of the file, or each error that stops it.
    pub(crate) pipeline: Result<FilePipeline, Vec<FileError>>,
    /// The secrets that the nodes name and that the project does not have.
    pub(crate) warnings: Vec<FileError>,
    /// The pipeline of the project with the key of the file.
    pub(crate) pipeline_id: Option<PipelineId>,
}

/// A pipeline file of the linked repository at a commit, as the server read it.
pub(crate) struct FileAt {
    /// The key of the pipeline.
    pub(crate) key: String,
    /// The path of the file.
    pub(crate) path: String,
    /// The format, from the extension of the path.
    pub(crate) format: Option<FileFormat>,
    /// The SHA of the blob. Empty for a file that the lookup refuses.
    pub(crate) blob_sha: String,
    /// The commit.
    pub(crate) commit_sha: String,
    /// The page of the file at the commit. Empty for a file that the lookup refuses.
    pub(crate) url: String,
    /// The text of the file.
    pub(crate) text: String,
    /// The pipeline of the file, or each error that stops it.
    pub(crate) pipeline: Result<FilePipeline, Vec<FileError>>,
    /// The secrets that the nodes name and that the project does not have.
    pub(crate) warnings: Vec<FileError>,
    /// The pipeline of the project with the key of the file.
    pub(crate) pipeline_id: Option<PipelineId>,
}

/// The pipeline files of the linked repository at a commit.
pub(crate) struct FilesAt {
    /// The commit.
    pub(crate) commit_sha: String,
    /// The files that the lookup takes, in the order of their keys.
    pub(crate) files: Vec<FileAt>,
    /// The files that the lookup refuses.
    pub(crate) rejected: Vec<Rejected>,
}

/// A pipeline as a file.
pub(crate) struct ExportedFile {
    /// The text of the file.
    pub(crate) content: String,
    /// The path of the file in the repository.
    pub(crate) path: String,
}

/// Reads and checks the text of a pipeline file at a path of the repository. Nothing changes. A
/// path that is not the path of a pipeline file, or a text larger than 1 MiB, gives
/// `INVALID_ARGUMENT`.
pub(crate) async fn validate(
    pool: &PgPool,
    project: ProjectId,
    path: &str,
    text: &str,
) -> Result<CheckedFile, Error> {
    // Find the key and the format of the path, and refuse a large text.
    let file = file_of(path)?;
    if text.len() > MAX_BYTES {
        return Err(Error::invalid("content", "must be at most 1 MiB"));
    }

    // Read the file, then find the secrets that the project does not have.
    let pipeline = pipeline_files::parse(&file.path, &file.key, file.format, text);
    let warnings = match &pipeline {
        Ok(checked) => {
            let names = checked.secret_names();
            let known = if names.is_empty() {
                Vec::new()
            } else {
                secrets::known(pool, project, &names).await?
            };
            pipeline_files::missing_secrets(&file.path, file.format, text, checked, &known)
        }
        Err(_) => Vec::new(),
    };

    // Find the pipeline of the project with the key of the file.
    let mut tx = pool.begin().await?;
    let pipeline_id = store::find_by_file_key(&mut tx, project, file.key.as_str()).await?;
    tx.commit().await?;
    Ok(CheckedFile {
        file,
        pipeline,
        warnings,
        pipeline_id,
    })
}

/// Writes a pipeline as a file in a format, with the path of the file in the folder of the
/// pipeline files. The key of the file comes from the name of the pipeline, and its `on` table
/// from its enabled Git triggers.
pub(crate) async fn export(
    pool: &PgPool,
    id: PipelineId,
    format: FileFormat,
) -> Result<ExportedFile, Error> {
    // Read the pipeline and the rules of its Git triggers.
    let mut tx = pool.begin().await?;
    let pipeline = store::find(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("pipeline"))?;
    tx.commit().await?;
    let rules = triggers::git_rules(pool, id).await?;

    // Write its document in the format, and the path of its file.
    let document = PipelineFile::of(&pipeline.name, &pipeline.nodes, &rules, &pipeline.checkout);
    Ok(ExportedFile {
        content: document.write(format),
        path: format!(
            "{FOLDER}/{}.{}",
            pipeline_key(&pipeline.name),
            format.extension()
        ),
    })
}

/// Returns the key and the format of the path of a pipeline file, or `INVALID_ARGUMENT`.
fn file_of(path: &str) -> Result<FileRef, Error> {
    let entry = TreeEntry {
        path: path.to_owned(),
        size: None,
    };
    let mut found = pipeline_files::lookup(&[entry]);
    if let Some(rejected) = found.rejected.pop() {
        let reason = format!("is refused: {}", rejected.error.message);
        return Err(Error::invalid("path", reason));
    }
    found.files.pop().ok_or_else(|| {
        let reason = "must be .scylla/pipeline.<ext> or .scylla/pipelines/<key>.<ext>, with the \
                      extension toml, yaml, yml or json";
        Error::invalid("path", reason)
    })
}

/// Reads the pipeline files of the linked repository of a project at a ref, or at the head of the
/// default branch, with the pipeline of each key and the secrets that the project does not have.
pub(crate) async fn list_files(
    ctx: &IntegrationContext,
    project: ProjectId,
    git_ref: Option<GitRef>,
) -> Result<FilesAt, Error> {
    // Read the files, then the secrets that the project has.
    let snapshot = files::read_at(ctx, project, git_ref).await?;
    let mut names: Vec<String> = Vec::new();
    for pipeline in snapshot
        .files
        .iter()
        .filter_map(|read| read.pipeline.as_ref().ok())
    {
        names.extend(pipeline.secret_names());
    }
    let known = if names.is_empty() {
        Vec::new()
    } else {
        secrets::known(&ctx.pool, project, &names).await?
    };

    // Give each file its warnings and its pipeline.
    let mut found = Vec::with_capacity(snapshot.files.len());
    let mut tx = ctx.pool.begin().await?;
    for read in snapshot.files {
        let key = read.file.key.as_str().to_owned();
        let pipeline_id = store::find_by_file_key(&mut tx, project, &key).await?;
        let warnings = match &read.pipeline {
            Ok(pipeline) => {
                let (path, format) = (&read.file.path, read.file.format);
                pipeline_files::missing_secrets(path, format, &read.text, pipeline, &known)
            }
            Err(_) => Vec::new(),
        };
        found.push(FileAt {
            key,
            path: read.file.path,
            format: Some(read.file.format),
            blob_sha: read.blob_sha,
            commit_sha: snapshot.commit_sha.clone(),
            url: read.url,
            text: read.text,
            pipeline: read.pipeline,
            warnings,
            pipeline_id,
        });
    }
    tx.commit().await?;
    Ok(FilesAt {
        commit_sha: snapshot.commit_sha,
        files: found,
        rejected: snapshot.rejected,
    })
}

/// Reads the file of a file pipeline at a ref, or at the head of the default branch. An editor
/// pipeline gives `NOT_A_FILE_PIPELINE`, and a commit with no file of its key gives `NOT_FOUND`.
pub(crate) async fn get_file(
    ctx: &IntegrationContext,
    id: PipelineId,
    git_ref: Option<GitRef>,
) -> Result<FileAt, Error> {
    // Refuse an editor pipeline.
    let pipeline = service::get(&ctx.pool, id)
        .await?
        .ok_or(Error::NotFound("pipeline"))?;
    let Some(origin) = pipeline.file else {
        let message = "the pipeline does not come from a file";
        return Err(Error::Rejected(Rejection::new(
            "NOT_A_FILE_PIPELINE",
            message,
        )));
    };

    // Take the file of the key at the commit, or the refusal of its files.
    let found = list_files(ctx, pipeline.project_id, git_ref).await?;
    if let Some(file) = found.files.into_iter().find(|file| file.key == origin.key) {
        return Ok(file);
    }
    let rejected = found
        .rejected
        .into_iter()
        .find(|rejected| rejected.key.as_ref().map(|key| key.as_str()) == Some(&origin.key))
        .ok_or(Error::NotFound("pipeline file"))?;
    let path = rejected.error.path.clone();
    Ok(FileAt {
        key: origin.key,
        format: path
            .rsplit_once('.')
            .and_then(|(_, extension)| FileFormat::of_extension(extension)),
        path,
        blob_sha: String::new(),
        commit_sha: found.commit_sha,
        url: String::new(),
        text: String::new(),
        pipeline: Err(vec![rejected.error]),
        warnings: Vec::new(),
        pipeline_id: Some(id),
    })
}

/// Sets the source of the pipelines of a project: the editor, or the files of its repository.
pub(crate) async fn set_source(
    ctx: &IntegrationContext,
    actor: ActorId,
    project: ProjectId,
    source: PipelineSource,
) -> Result<(), Error> {
    files::set_source(ctx, actor, project, source).await
}
