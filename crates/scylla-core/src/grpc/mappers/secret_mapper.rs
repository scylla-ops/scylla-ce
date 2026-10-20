//! Wire to command, command outcome to wire. The handler holds none of it.

use crate::application::secret::{CreateSecret, DeleteSecret, ListSecrets};
use crate::grpc::convert::{Parse, id, ts, valid, wrap};
use scylla_domain::domain::secret::{
    Secret as DomainSecret, SecretDescription, SecretName, SecretValue,
};
use scylla_proto::secret::v1::{
    CreateSecretRequest, DeleteSecretRequest, ListSecretsRequest, Secret,
};
use tonic::Status;

pub fn secret_to_proto(secret: &DomainSecret) -> Secret {
    Secret {
        secret_id: wrap(secret.id().to_string()),
        project_id: wrap(secret.project_id().to_string()),
        name: secret.name().as_str().to_string(),
        description: secret.description().to_string(),
        created_at: ts(secret.created_at()),
        updated_at: ts(secret.updated_at()),
    }
}

impl Parse for CreateSecretRequest {
    type Into = CreateSecret;

    fn parse(self) -> Result<CreateSecret, Status> {
        Ok(CreateSecret {
            project_id: id(self.project_id, "project_id")?,
            name: valid(self.name, SecretName::new)?,
            description: valid(self.description, SecretDescription::new)?,
            value: valid(self.value, SecretValue::new)?,
        })
    }
}

parse!(ListSecretsRequest => ListSecrets { project_id: id(project_id) });

parse!(DeleteSecretRequest => DeleteSecret { id: id(secret_id) });

#[cfg(test)]
mod tests {
    use super::*;
    use scylla_proto::common::v1 as common;
    use tonic::Code;

    #[test]
    fn a_create_request_becomes_a_command_with_validated_fields() {
        let command = CreateSecretRequest {
            project_id: wrap("proj-1"),
            name: " DB_PASSWORD ".into(),
            value: "hunter2".into(),
            description: "db".into(),
        }
        .parse()
        .unwrap();

        assert_eq!(command.project_id.as_str(), "proj-1");
        assert_eq!(command.name.as_str(), "DB_PASSWORD");
        assert_eq!(command.value.as_str(), "hunter2");
        assert_eq!(command.description.as_str(), "db");
        assert!(!format!("{command:?}").contains("hunter2"));
    }

    #[test]
    fn an_oversized_value_is_an_invalid_argument() {
        let Err(err) = CreateSecretRequest {
            project_id: wrap("proj-1"),
            name: "BIG".into(),
            value: "a".repeat(65_537),
            description: String::new(),
        }
        .parse() else {
            panic!("an oversized value must not parse");
        };

        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn a_missing_project_id_is_an_invalid_argument() {
        let err = ListSecretsRequest {
            project_id: None::<common::ProjectId>,
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing project_id");
    }

    #[test]
    fn an_invalid_name_is_an_invalid_argument() {
        let Err(err) = CreateSecretRequest {
            project_id: wrap("proj-1"),
            name: "not a name".into(),
            value: "v".into(),
            description: String::new(),
        }
        .parse() else {
            panic!("an invalid name must not parse");
        };

        assert_eq!(err.code(), Code::InvalidArgument);
    }
}
