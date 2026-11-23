//! The conversions of the library between the domain and the protos. A bad value gives
//! `INVALID_ARGUMENT` with its path, for example `parameters[1].key`.

use scylla_domain::projects::pipelines::NodeError;
use scylla_domain::tenancy::library::checks;
use scylla_domain::tenancy::library::{
    Author, Fragment, NewOrigin, NewVersion, Origin, Parameter, ParameterValue, Usage, Version,
};
use scylla_proto::library::v1 as pb;
use scylla_proto::pipelines::v1::Node;

use crate::api::parse::{self, ToProto};
use crate::features::projects::pipelines::convert;
use crate::Error;

/// Reads and checks the content of a version: its nodes, its parameters and what changed.
pub(super) fn version(
    nodes: Vec<Node>,
    parameters: Vec<pb::FragmentParameter>,
    changes: Vec<String>,
) -> Result<NewVersion, Error> {
    let nodes = convert::nodes(nodes)?.into_vec();
    let parameters = parameters
        .into_iter()
        .enumerate()
        .map(|(index, raw)| parameter(raw, index))
        .collect::<Result<_, _>>()?;
    Ok(NewVersion {
        parameters: checks::parameters(parameters, &nodes).map_err(invalid)?,
        changes: checks::changes(changes).map_err(invalid)?,
        nodes,
    })
}

/// Reads the description of a request.
pub(super) fn description(raw: String) -> Result<String, Error> {
    checks::description(raw).map_err(invalid)
}

/// Reads the tags of a request.
pub(super) fn tags(raw: Vec<String>) -> Result<Vec<String>, Error> {
    checks::tags(raw).map_err(invalid)
}

/// Reads the origin of a new fragment.
pub(super) fn origin(raw: pb::NewFragmentOrigin) -> Result<NewOrigin, Error> {
    let origin = NewOrigin {
        registry: raw.registry,
        package_id: raw.package_id,
        package_name: raw.package_name,
        version: raw.version,
        linked: raw.linked,
    };
    checks::origin(origin).map_err(invalid)
}

/// Reads one parameter of a request. A literal keeps its default, and a secret has none.
fn parameter(raw: pb::FragmentParameter, index: usize) -> Result<Parameter, Error> {
    let at = |field: &str| format!("parameters[{index}].{field}");
    let value = match pb::ParameterKind::try_from(raw.kind) {
        Ok(pb::ParameterKind::Literal) => ParameterValue::Literal(raw.default_value),
        Ok(pb::ParameterKind::Secret) if raw.default_value.is_empty() => ParameterValue::Secret,
        Ok(pb::ParameterKind::Secret) => {
            return Err(Error::invalid(
                at("default_value"),
                "must be empty for a secret",
            ));
        }
        _ => return Err(Error::invalid(at("kind"), "must be literal or secret")),
    };
    Ok(Parameter {
        key: raw.key,
        value,
        description: raw.description,
    })
}

/// Makes the request error of a bad value.
fn invalid(error: NodeError) -> Error {
    Error::invalid(error.path, error.error.to_string())
}

impl ToProto<pb::Fragment> for Fragment {
    fn to_proto(self) -> pb::Fragment {
        pb::Fragment {
            id: Some(self.id.to_proto()),
            workspace_id: Some(self.workspace_id.to_proto()),
            name: self.name.as_str().to_owned(),
            description: self.description,
            tags: self.tags,
            author: Some(self.author.to_proto()),
            versions: self.versions.into_iter().map(ToProto::to_proto).collect(),
            favorite: self.favorite,
            usages: self.usages.into_iter().map(ToProto::to_proto).collect(),
            created_at: Some(parse::timestamp(self.created_at)),
            origin: self.origin.map(ToProto::to_proto),
        }
    }
}

impl ToProto<pb::FragmentOrigin> for Origin {
    fn to_proto(self) -> pb::FragmentOrigin {
        pb::FragmentOrigin {
            registry: self.registry,
            package_id: self.package_id,
            package_name: self.package_name,
            linked: self.linked,
            base_version: self.base_version,
            skipped: self.skipped,
            dismissed: self.dismissed,
            added_at: Some(parse::timestamp(self.added_at)),
        }
    }
}

impl ToProto<pb::FragmentAuthor> for Author {
    fn to_proto(self) -> pb::FragmentAuthor {
        pb::FragmentAuthor {
            id: Some(self.id.to_proto()),
            name: self.name,
        }
    }
}

impl ToProto<pb::FragmentVersion> for Version {
    fn to_proto(self) -> pb::FragmentVersion {
        pb::FragmentVersion {
            number: self.number,
            nodes: self.nodes.into_iter().map(ToProto::to_proto).collect(),
            parameters: self.parameters.into_iter().map(ToProto::to_proto).collect(),
            changes: self.changes,
            author: Some(self.author.to_proto()),
            published_at: Some(parse::timestamp(self.published_at)),
        }
    }
}

impl ToProto<pb::FragmentParameter> for Parameter {
    fn to_proto(self) -> pb::FragmentParameter {
        let (kind, default_value) = match self.value {
            ParameterValue::Literal(default) => (pb::ParameterKind::Literal, default),
            ParameterValue::Secret => (pb::ParameterKind::Secret, String::new()),
        };
        pb::FragmentParameter {
            key: self.key,
            kind: kind.into(),
            default_value,
            description: self.description,
        }
    }
}

impl ToProto<pb::FragmentUsage> for Usage {
    fn to_proto(self) -> pb::FragmentUsage {
        pb::FragmentUsage {
            pipeline_id: Some(self.pipeline_id.to_proto()),
            pipeline_name: self.pipeline_name,
            project_id: Some(self.project_id.to_proto()),
            project_name: self.project_name,
            version: self.version,
            values: self.values.into_iter().collect(),
        }
    }
}
