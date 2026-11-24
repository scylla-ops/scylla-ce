//! The SQL of the `fragment_versions` table, and the JSON form of the parameters. Each function
//! runs in the transaction of the caller.

use std::collections::HashMap;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::library::{
    Author, FragmentId, NewVersion, Parameter, ParameterValue, Version,
};
use scylla_extension::Tx;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::features::projects::pipelines::json;
use crate::Error;

/// Which versions keep their nodes in a read.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Nodes {
    /// Each version.
    All,
    /// The latest version of each fragment only.
    Latest,
}

/// A parameter in JSON.
#[derive(Serialize, Deserialize)]
struct ParameterJson {
    /// The name of the variable.
    key: String,
    /// The default of a literal, or `None` for a secret.
    literal: Option<String>,
    /// The description.
    description: String,
}

/// Returns the number of the latest version of a fragment, or 0 with no version.
pub(crate) async fn latest(tx: &mut Tx<'_>, id: FragmentId) -> Result<u32, sqlx::Error> {
    let number = sqlx::query_scalar!(
        "SELECT max(number) FROM fragment_versions WHERE fragment_id = $1",
        id.to_db(),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(number
        .and_then(|number| u32::try_from(number).ok())
        .unwrap_or(0))
}

/// Inserts a version of a fragment.
pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    id: FragmentId,
    number: u32,
    created_by: ActorId,
    version: &NewVersion,
) -> Result<(), Error> {
    let number = i32::try_from(number).map_err(|_| Error::invalid("version", "is too large"))?;
    sqlx::query!(
        "INSERT INTO fragment_versions (fragment_id, number, nodes, parameters, changes, created_by)
         VALUES ($1, $2, $3, $4, $5, $6)",
        id.to_db(),
        number,
        json::nodes(&version.nodes),
        parameters_json(&version.parameters),
        &version.changes,
        created_by.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the versions of fragments by fragment, the oldest first.
pub(crate) async fn of(
    tx: &mut Tx<'_>,
    ids: &[Uuid],
    nodes: Nodes,
) -> Result<HashMap<Uuid, Vec<Version>>, Error> {
    let rows = sqlx::query!(
        r#"SELECT v.fragment_id, v.number, v.parameters, v.changes, v.created_by, v.published_at,
                  u.display_name AS "author_name?",
                  CASE WHEN $2 OR v.number = (SELECT max(number) FROM fragment_versions l
                                              WHERE l.fragment_id = v.fragment_id)
                       THEN v.nodes ELSE '[]'::jsonb END AS "nodes!"
           FROM fragment_versions v LEFT JOIN users u ON u.id = v.created_by
           WHERE v.fragment_id = ANY($1)
           ORDER BY v.fragment_id, v.number"#,
        ids,
        nodes == Nodes::All,
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut versions: HashMap<Uuid, Vec<Version>> = HashMap::new();
    for row in rows {
        let version = to_version(
            row.number,
            row.nodes,
            row.parameters,
            row.changes,
            Author {
                id: ActorId::from_db(row.created_by),
                name: row.author_name.unwrap_or_default(),
            },
            row.published_at,
        )?;
        versions.entry(row.fragment_id).or_default().push(version);
    }
    Ok(versions)
}

/// Makes a version from its columns. Bad data in the database gives `INTERNAL`.
fn to_version(
    number: i32,
    nodes: Value,
    parameters: Value,
    changes: Vec<String>,
    author: Author,
    published_at: OffsetDateTime,
) -> Result<Version, Error> {
    let bad = |error: serde_json::Error| Error::Internal(format!("bad parameters: {error}"));
    let parameters: Vec<ParameterJson> = serde_json::from_value(parameters).map_err(bad)?;
    Ok(Version {
        number: u32::try_from(number).unwrap_or_default(),
        nodes: json::read_nodes(nodes)?,
        parameters: parameters.into_iter().map(parameter_of).collect(),
        changes,
        author,
        published_at,
    })
}

/// Returns the JSON of parameters.
fn parameters_json(parameters: &[Parameter]) -> Value {
    let items: Vec<ParameterJson> = parameters
        .iter()
        .map(|parameter| ParameterJson {
            key: parameter.key.clone(),
            literal: match &parameter.value {
                ParameterValue::Literal(default) => Some(default.clone()),
                ParameterValue::Secret => None,
            },
            description: parameter.description.clone(),
        })
        .collect();
    serde_json::to_value(items).unwrap_or(Value::Null)
}

/// Makes a parameter from its JSON.
fn parameter_of(json: ParameterJson) -> Parameter {
    Parameter {
        key: json.key,
        value: json
            .literal
            .map_or(ParameterValue::Secret, ParameterValue::Literal),
        description: json.description,
    }
}
