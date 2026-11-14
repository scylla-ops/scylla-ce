//! The files that a pull request changes, as the provider lists them: the approval of a run from a
//! fork warns when the pull request changes a file in `.scylla/`.

use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::ProjectId;

use super::commits::Failure;
use super::{github, gitlab, store};
use crate::features::integrations::connections::credentials;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// The folder of the pipeline files.
const FOLDER: &str = ".scylla/";

/// Returns `true` when a pull request of the linked repository of a project adds, changes,
/// removes or renames a file in `.scylla/`. A failed read gives `true`, so the approval warns.
pub(crate) async fn changes_pipeline_files(
    ctx: &IntegrationContext,
    project: ProjectId,
    number: u32,
) -> bool {
    match paths(ctx, project, number).await {
        Ok(paths) => paths.iter().any(|path| path.starts_with(FOLDER)),
        Err(error) => {
            tracing::warn!(%project, number, %error, "the files of a pull request were not read");
            true
        }
    }
}

/// Returns the paths that a pull request changes, with the old path of each renamed file.
async fn paths(
    ctx: &IntegrationContext,
    project: ProjectId,
    number: u32,
) -> Result<Vec<String>, Error> {
    // Read the repository of the project and its connection.
    let mut tx = ctx.pool.begin().await?;
    let target = store::target(&mut tx, project).await?;
    tx.commit().await?;
    let target = target.ok_or(Error::NotFound("repository link"))?;

    // Ask the provider for the files of the pull request.
    let changed = match target.provider {
        VcsProvider::GitHub => github::changed_paths(ctx, &target, number).await,
        VcsProvider::GitLab => gitlab::changed_paths(ctx, &target, number).await,
    };
    changed.map_err(|failure| match failure {
        Failure::Server(error) => error,
        Failure::Provider(error) => credentials::provider_failed(target.provider, error),
    })
}
