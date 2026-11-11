//! The logic of the pipeline files: the check of a file that a client sends, and the export of a
//! pipeline as a file.

use scylla_domain::integrations::reports::pipeline_key;
use scylla_domain::projects::git_rules::GitRules;
use scylla_domain::projects::pipeline_files::{
    self, CheckoutSettings, FileError, FileFormat, FilePipeline, FileRef, PipelineFile, TreeEntry,
    MAX_BYTES,
};
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::ProjectId;
use sqlx::PgPool;

use super::store;
use crate::features::projects::secrets;
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
/// pipeline files. The key of the file comes from the name of the pipeline.
pub(crate) async fn export(
    pool: &PgPool,
    id: PipelineId,
    format: FileFormat,
) -> Result<ExportedFile, Error> {
    // Read the pipeline.
    let mut tx = pool.begin().await?;
    let pipeline = store::find(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("pipeline"))?;
    tx.commit().await?;

    // Write its document in the format, and the path of its file.
    let document = PipelineFile::of(
        &pipeline.name,
        &pipeline.nodes,
        &GitRules::default(),
        &CheckoutSettings::default(),
    );
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
