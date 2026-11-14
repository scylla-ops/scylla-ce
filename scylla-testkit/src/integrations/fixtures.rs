//! The rows of the Git integration that a test needs before the routes that make them: a
//! connection, a repository link, the source of a job and a Git trigger. The SQL runs at run
//! time, and each helper returns the text of the id of its row.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use serde_json::Value;

use crate::TestApp;

/// The repository of a linked project.
pub struct LinkRow<'a> {
    /// The project, as the text of its id.
    pub project: &'a str,
    /// The connection, as the text of its id.
    pub connection: &'a str,
    /// The id of the repository on the provider.
    pub external_id: &'a str,
    /// The full name, such as `octo/app`.
    pub full_name: &'a str,
    /// The default branch.
    pub default_branch: &'a str,
}

impl TestApp {
    /// Inserts an active connection of an installation of the GitHub App, for each repository of
    /// an organization.
    pub async fn insert_github_connection(
        &self,
        workspace: WorkspaceId,
        author: ActorId,
        installation: u64,
        account: &str,
    ) -> String {
        let id = new_id();
        sqlx::query(
            "INSERT INTO vcs_connections (id, workspace_id, provider, auth_kind, external_id,
                                          account, account_kind, all_repositories, created_by)
             VALUES ($1::uuid, $2, 'github', 'app_installation', $3, $4, 'organization', true, $5)",
        )
        .bind(&id)
        .bind(workspace.to_db())
        .bind(installation.to_string())
        .bind(account)
        .bind(author.to_db())
        .execute(&self.pool)
        .await
        .expect("insert a GitHub connection");
        id
    }

    /// Inserts an active connection of a GitLab service account, with its token sealed with the
    /// id of the row.
    pub async fn insert_gitlab_connection(
        &self,
        workspace: WorkspaceId,
        author: ActorId,
        account: &str,
        token: &str,
    ) -> String {
        let id = uuid::Uuid::now_v7();
        let sealer = self
            .sealer
            .as_ref()
            .expect("a server with a key of the secrets");
        let sealed = sealer.seal_with(token.as_bytes(), id.as_bytes());
        sqlx::query(
            "INSERT INTO vcs_connections (id, workspace_id, provider, auth_kind, external_id,
                                          account, account_kind, sealed_token, created_by)
             VALUES ($1, $2, 'gitlab', 'access_token', $3, $3, 'service_account', $4, $5)",
        )
        .bind(id)
        .bind(workspace.to_db())
        .bind(account)
        .bind(sealed)
        .bind(author.to_db())
        .execute(&self.pool)
        .await
        .expect("insert a GitLab connection");
        id.to_string()
    }

    /// Links a project to a public repository of a connection, with the webhook mode, the
    /// reports on and the editor as the pipeline source.
    pub async fn insert_repository_link(&self, link: &LinkRow<'_>) {
        sqlx::query(
            "INSERT INTO repository_links (project_id, workspace_id, connection_id, external_id,
                                           full_name, web_url, default_branch, private,
                                           created_by)
             SELECT p.id, p.workspace_id, c.id, $3, $4,
                    CASE c.provider WHEN 'github' THEN 'https://github.com/'
                                    ELSE 'https://gitlab.com/' END || $4,
                    $5, false, c.created_by
             FROM projects p, vcs_connections c
             WHERE p.id = $1::uuid AND c.id = $2::uuid",
        )
        .bind(link.project)
        .bind(link.connection)
        .bind(link.external_id)
        .bind(link.full_name)
        .bind(link.default_branch)
        .execute(&self.pool)
        .await
        .expect("insert a repository link");
    }

    /// Gives a job the source of a commit on a branch, from the repository of its project (or
    /// `octo/app` on GitHub with no link). `event` is `push`, `pull_request`, `tag` or `manual`.
    pub async fn insert_job_source(&self, job: &str, sha: &str, branch: &str, event: &str) {
        sqlx::query(
            "INSERT INTO job_sources (job_id, pipeline_id, provider, repository, url, sha, ref,
                                      branch, author_login, message_title, event, automatic)
             SELECT j.id, j.pipeline_id, COALESCE(c.provider, 'github'),
                    COALESCE(l.full_name, 'octo/app'),
                    COALESCE(l.web_url, 'https://github.com/octo/app'),
                    $2, 'refs/heads/' || $3, $3, 'octocat', 'Test commit', $4,
                    $4 IN ('push', 'pull_request', 'tag')
             FROM jobs j
             LEFT JOIN repository_links l ON l.project_id = j.project_id
             LEFT JOIN vcs_connections c ON c.id = l.connection_id
             WHERE j.id = $1::uuid",
        )
        .bind(job)
        .bind(sha)
        .bind(branch)
        .bind(event)
        .execute(&self.pool)
        .await
        .expect("insert a job source");
    }

    /// Inserts an enabled Git trigger of a pipeline: `kind` is `push`, `pull_request` or `tag`,
    /// and `rules` is the JSON of its rules.
    pub async fn insert_git_trigger(
        &self,
        pipeline: &str,
        name: &str,
        kind: &str,
        rules: Value,
    ) -> String {
        let id = new_id();
        sqlx::query(
            "INSERT INTO triggers (id, pipeline_id, project_id, workspace_id, created_by, name,
                                   kind, rules, inputs)
             SELECT $1::uuid, p.id, p.project_id, p.workspace_id, p.created_by, $3, $4, $5, '[]'
             FROM pipelines p
             WHERE p.id = $2::uuid",
        )
        .bind(&id)
        .bind(pipeline)
        .bind(name)
        .bind(kind)
        .bind(rules)
        .execute(&self.pool)
        .await
        .expect("insert a Git trigger");
        id
    }
}

/// Makes the text of a new id.
fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}
