//! The environment of the nodes of a job at its dispatch: the values of its secrets, and the
//! variables of its commit. A job from a fork gets no secret value.

use scylla_domain::integrations::sources::JobSource;
use scylla_domain::projects::jobs::Job;
use scylla_domain::projects::pipelines::EnvValue;
use sqlx::PgPool;

use crate::features::projects::secrets;
use crate::infra::secret::Sealer;

/// A variable of a node to run, with its value.
pub(crate) struct ResolvedEnv {
    /// The name.
    pub(crate) name: String,
    /// The value.
    pub(crate) value: String,
    /// `true` for the value of a secret.
    pub(crate) masked: bool,
}

/// Opens the environment of each node of a job, and adds the `SCYLLA_GIT_*` variables of its
/// source, not masked. A secret of a job from a fork gets an empty value. Returns the reason of a
/// failure.
pub(crate) async fn resolve(
    pool: &PgPool,
    sealer: Option<&Sealer>,
    job: &Job,
) -> Result<Vec<Vec<ResolvedEnv>>, String> {
    // Read the variables of the commit.
    let fork = job.source.as_ref().is_some_and(JobSource::from_fork);
    let git = job
        .source
        .as_ref()
        .map(JobSource::variables)
        .unwrap_or_default();
    let mut all = Vec::with_capacity(job.nodes.len());
    for node in &job.nodes {
        // Open the variables of the node.
        let mut env = Vec::with_capacity(node.env.len() + git.len());
        for var in &node.env {
            let (value, masked) = match &var.value {
                EnvValue::Literal(text) => (text.clone(), false),
                EnvValue::Secret(_) if fork => (String::new(), false),
                EnvValue::Secret(name) => {
                    let value = secrets::reveal(pool, sealer, job.project_id, name)
                        .await
                        .map_err(|error| format!("secret {}: {error}", name.as_str()))?;
                    (value.expose().to_owned(), true)
                }
            };
            env.push(ResolvedEnv {
                name: var.name.clone(),
                value,
                masked,
            });
        }

        // Add the variables of the commit.
        for var in &git {
            if let EnvValue::Literal(text) = &var.value {
                env.push(ResolvedEnv {
                    name: var.name.clone(),
                    value: text.clone(),
                    masked: false,
                });
            }
        }
        all.push(env);
    }
    Ok(all)
}
