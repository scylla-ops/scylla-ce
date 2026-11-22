//! The fragment links of a pipeline: their proto form and their JSON form in
//! `pipelines.fragment_links`.

use std::collections::BTreeMap;

use scylla_domain::projects::pipelines::{FragmentLink, NodeId};
use scylla_domain::tenancy::library::FragmentId;
use scylla_proto::pipelines::v1 as pb;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::parse::{FromProto, ToProto};
use crate::Error;

/// A link in JSON.
#[derive(Serialize, Deserialize)]
struct LinkJson {
    /// The id of the block.
    block_id: String,
    /// The id of the fragment, as text.
    fragment_id: String,
    /// The version of the fragment.
    version: u32,
    /// The values of the parameters.
    values: BTreeMap<String, String>,
    /// The nodes of the block.
    node_ids: Vec<String>,
    /// The version that the pipeline chose not to take.
    skipped_version: Option<u32>,
}

/// Reads the links of a request. Their checks against the nodes come with the change.
pub(crate) fn read(links: Vec<pb::FragmentLink>) -> Result<Vec<FragmentLink>, Error> {
    links
        .into_iter()
        .enumerate()
        .map(|(index, link)| {
            let at = |field: &str| format!("fragment_links[{index}].{field}");
            let node = |raw: String, field: String| {
                NodeId::parse(raw).map_err(|error| Error::invalid(field, error.to_string()))
            };
            let fragment = link
                .fragment_id
                .ok_or_else(|| Error::invalid(at("fragment_id"), "is required"))?;
            let fragment_id = FragmentId::from_proto(fragment)
                .map_err(|_| Error::invalid(at("fragment_id"), "must be an id"))?;
            let node_ids = link
                .node_ids
                .into_iter()
                .enumerate()
                .map(|(position, id)| node(id, at(&format!("node_ids[{position}]"))))
                .collect::<Result<_, _>>()?;
            Ok(FragmentLink {
                block_id: node(link.block_id, at("block_id"))?,
                fragment_id,
                version: link.version,
                values: link.values.into_iter().collect(),
                node_ids,
                skipped_version: link.skipped_version,
            })
        })
        .collect()
}

/// Returns the JSON of links.
pub(crate) fn json(links: &[FragmentLink]) -> Value {
    let items: Vec<LinkJson> = links
        .iter()
        .map(|link| LinkJson {
            block_id: link.block_id.as_str().to_owned(),
            fragment_id: link.fragment_id.to_string(),
            version: link.version,
            values: link.values.clone(),
            node_ids: link
                .node_ids
                .iter()
                .map(|id| id.as_str().to_owned())
                .collect(),
            skipped_version: link.skipped_version,
        })
        .collect();
    serde_json::to_value(items).unwrap_or(Value::Null)
}

/// Reads links from their JSON. Bad data in the database gives `INTERNAL`.
pub(crate) fn read_json(value: Value) -> Result<Vec<FragmentLink>, Error> {
    let bad = |error: String| Error::Internal(format!("bad fragment links of a pipeline: {error}"));
    let items: Vec<LinkJson> = serde_json::from_value(value).map_err(|e| bad(e.to_string()))?;
    let node = |raw: String| NodeId::parse(raw).map_err(|e| bad(e.to_string()));
    items
        .into_iter()
        .map(|item| {
            Ok(FragmentLink {
                block_id: node(item.block_id)?,
                fragment_id: FragmentId::parse(&item.fragment_id)
                    .map_err(|e| bad(e.to_string()))?,
                version: item.version,
                values: item.values,
                node_ids: item
                    .node_ids
                    .into_iter()
                    .map(node)
                    .collect::<Result<_, _>>()?,
                skipped_version: item.skipped_version,
            })
        })
        .collect()
}

impl ToProto<pb::FragmentLink> for FragmentLink {
    fn to_proto(self) -> pb::FragmentLink {
        pb::FragmentLink {
            block_id: self.block_id.as_str().to_owned(),
            fragment_id: Some(self.fragment_id.to_proto()),
            version: self.version,
            values: self.values.into_iter().collect(),
            node_ids: self
                .node_ids
                .iter()
                .map(|id| id.as_str().to_owned())
                .collect(),
            skipped_version: self.skipped_version,
        }
    }
}
