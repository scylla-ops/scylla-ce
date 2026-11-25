//! The keys of the pipelines of a project. A file pipeline and an editor pipeline share the keys,
//! which name their reports: a new editor pipeline cannot take the key of a file, and a file that
//! takes the key of an editor pipeline wins.

use std::collections::HashSet;

use scylla_domain::integrations::reports::pipeline_key;
use scylla_domain::projects::pipelines::{PipelineId, PipelineName};
use scylla_domain::projects::ProjectId;
use scylla_extension::{Rejection, Tx};

use super::store;
use crate::Error;

/// The code of a key that a file pipeline has.
const KEY_TAKEN: &str = "PIPELINE_KEY_TAKEN";

/// Refuses the key of a name when an active file pipeline of the default branch has it, with
/// `PIPELINE_KEY_TAKEN` and the key. The link of the project stays locked, so a sync waits.
pub(super) async fn refuse_taken(
    tx: &mut Tx<'_>,
    project: ProjectId,
    name: &PipelineName,
) -> Result<(), Error> {
    let key = pipeline_key(name.as_str());
    if store::file_has_key(tx, project, &key).await? {
        let message = "a pipeline file of the repository has the key of this name";
        let rejection = Rejection::new(KEY_TAKEN, message).detail("key", &key);
        return Err(Error::Rejected(rejection));
    }
    Ok(())
}

/// Refuses a new name of a pipeline whose key is a new key that a file pipeline has. Returns
/// `true` when the key changes: the new key is free.
pub(super) async fn check_rename(
    tx: &mut Tx<'_>,
    id: PipelineId,
    name: Option<&PipelineName>,
) -> Result<bool, Error> {
    let Some(name) = name else {
        return Ok(false);
    };
    let pipeline = store::find(tx, id)
        .await?
        .ok_or(Error::NotFound("pipeline"))?;
    if pipeline_key(name.as_str()) == pipeline.key() {
        return Ok(false);
    }
    refuse_taken(tx, pipeline.project_id, name).await?;
    Ok(true)
}

/// Marks each active editor pipeline of a project whose key an active file pipeline of the
/// default branch has, and frees the others.
pub(crate) async fn mark_taken(tx: &mut Tx<'_>, project: ProjectId) -> Result<(), Error> {
    // Find the pipelines whose mark changes.
    let files: HashSet<String> = store::file_keys(tx, project).await?.into_iter().collect();
    let (mut taken, mut free) = (Vec::new(), Vec::new());
    for pipeline in store::editor_keys(tx, project).await? {
        match (
            files.contains(&pipeline_key(&pipeline.name)),
            pipeline.taken,
        ) {
            (true, false) => taken.push(pipeline.id),
            (false, true) => free.push(pipeline.id),
            _ => {}
        }
    }

    // Change their marks.
    if !taken.is_empty() {
        store::mark_key_taken(tx, &taken, true).await?;
    }
    if !free.is_empty() {
        store::mark_key_taken(tx, &free, false).await?;
    }
    Ok(())
}
