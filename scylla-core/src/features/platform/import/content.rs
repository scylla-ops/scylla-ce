//! The import of the projects with their roles, their secrets sealed again with the key of this
//! base, and their pipelines.

use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::secrets::{SecretId, SecretName, SecretValue};
use scylla_domain::projects::{Project, ProjectDescription, ProjectId};
use scylla_domain::Name;

use super::legacy::Legacy;
use super::service::Run;
use super::{ids, nodes};
use crate::features::projects::pipelines::json;
use crate::Error;

/// Imports the projects of the imported workspaces, then their secrets and their pipelines.
pub(super) async fn import(run: &mut Run<'_>, legacy: &Legacy) -> Result<(), Error> {
    projects(run, legacy).await?;
    secrets(run, legacy).await?;
    pipelines(run, legacy).await
}

/// Imports the projects, with their old ids, and their roles.
async fn projects(run: &mut Run<'_>, legacy: &Legacy) -> Result<(), Error> {
    for old in &legacy.projects {
        let Some((workspace, author)) = run.workspaces.get(&old.organization_id).copied() else {
            run.report.skipped.push(format!(
                "project {}: its organization is not imported",
                old.name
            ));
            continue;
        };
        let id = ids::of_ulid(&old.id).map_or_else(ProjectId::generate, ProjectId::from_db);
        let name = Name::parse(old.name.clone())
            .or_else(|_| Name::parse("Project".to_owned()))
            .expect("a valid name");
        let description = ProjectDescription::parse(old.description.clone().unwrap_or_default())
            .unwrap_or_default();
        sqlx::query(
            "INSERT INTO projects (id, workspace_id, created_by, name, description, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id.to_db())
        .bind(workspace.to_db())
        .bind(author.to_db())
        .bind(name.as_str())
        .bind(description.as_str())
        .bind(old.created_at)
        .execute(run.pool)
        .await?;
        run.projects.insert(old.id.clone(), (id, workspace, author));
        run.report.projects += 1;

        // Keep the roles of the users on the project.
        for grant in legacy.grants.iter().filter(|grant| {
            grant.scope_kind == "project"
                && grant.scope_id == old.id
                && grant.principal_kind == "user"
        }) {
            let Some(user) = run.users.get(&grant.principal_id).copied() else {
                continue;
            };
            let role = match (grant.role_key.as_deref(), grant.role_kind.as_str()) {
                (Some("project-admin"), _) | (None, "admin") => Project::ADMIN.of(id, user),
                (Some("project-developer"), _) | (None, "member") => Project::EDITOR.of(id, user),
                (Some("project-viewer"), _) => Project::VIEWER.of(id, user),
                _ => continue,
            };
            run.roles.push(role);
        }
    }
    Ok(())
}

/// Opens each secret with the old key, and seals it again with the key of this base and the id
/// of the secret.
async fn secrets(run: &mut Run<'_>, legacy: &Legacy) -> Result<(), Error> {
    for old in &legacy.secrets {
        let Some((project, _, author)) = run.projects.get(&old.project_id).copied() else {
            continue;
        };
        let (Some(old_sealer), Some(sealer)) = (run.old_sealer, run.sealer) else {
            run.report.skipped.push(format!(
                "secret {}: the import needs the old key and the new key",
                old.name
            ));
            continue;
        };
        let opened = old_sealer
            .open(&old.encrypted_value)
            .map_err(|reason| reason.to_string());
        let parsed = opened.and_then(|value| {
            let value = SecretValue::parse(value).map_err(|error| error.to_string())?;
            let name = SecretName::parse(old.name.clone()).map_err(|error| error.to_string())?;
            Ok((name, value))
        });
        let (name, value) = match parsed {
            Ok(parsed) => parsed,
            Err(reason) => {
                run.report
                    .skipped
                    .push(format!("secret {}: {reason}", old.name));
                continue;
            }
        };
        let id = ids::of_ulid(&old.id).map_or_else(SecretId::generate, SecretId::from_db);
        let sealed = sealer.seal_with(value.expose().as_bytes(), &id.to_bytes());
        sqlx::query(
            "INSERT INTO secrets (id, project_id, name, sealed_value, created_by) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(id.to_db())
        .bind(project.to_db())
        .bind(name.as_str())
        .bind(sealed)
        .bind(author.to_db())
        .execute(run.pool)
        .await?;
        run.report.secrets += 1;
    }
    Ok(())
}

/// Imports each pipeline whose nodes fit the rules of this base.
async fn pipelines(run: &mut Run<'_>, legacy: &Legacy) -> Result<(), Error> {
    for old in &legacy.pipelines {
        let Some((project, workspace, author)) = run.projects.get(&old.project_id).copied() else {
            continue;
        };
        let checked = match nodes::read(old.nodes.clone()) {
            Ok(checked) => checked,
            Err(reason) => {
                run.report
                    .skipped
                    .push(format!("pipeline {}: {reason}", old.name));
                continue;
            }
        };
        let id = ids::of_ulid(&old.id).map_or_else(PipelineId::generate, PipelineId::from_db);
        let name = Name::parse(old.name.clone())
            .or_else(|_| Name::parse("Pipeline".to_owned()))
            .expect("a valid name");
        sqlx::query(
            "INSERT INTO pipelines (id, project_id, workspace_id, created_by, name, nodes, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id.to_db())
        .bind(project.to_db())
        .bind(workspace.to_db())
        .bind(author.to_db())
        .bind(name.as_str())
        .bind(json::nodes(checked.as_slice()))
        .bind(old.created_at)
        .execute(run.pool)
        .await?;
        run.report.pipelines += 1;
    }
    Ok(())
}
