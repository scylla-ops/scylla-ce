//! The SpiceDB client: permission checks, reverse lookups and the schema.

use scylla_domain::authz::Object;
use scylla_extension::AccessError;
use spicedb_grpc_tonic::v1::check_bulk_permissions_pair::Response as Answer;
use spicedb_grpc_tonic::v1::consistency::Requirement;
use spicedb_grpc_tonic::v1::permissions_service_client::PermissionsServiceClient;
use spicedb_grpc_tonic::v1::schema_service_client::SchemaServiceClient;
use spicedb_grpc_tonic::v1::{
    check_permission_response::Permissionship, CheckBulkPermissionsRequest,
    CheckBulkPermissionsRequestItem, Consistency, LookupPermissionship, LookupResourcesRequest,
    ObjectReference, SubjectReference, WriteSchemaRequest, ZedToken,
};
use tonic::metadata::{Ascii, MetadataValue};
use tonic::service::interceptor::InterceptedService;
use tonic::service::Interceptor;
use tonic::transport::{Channel, Endpoint};
use tonic::{Request, Status};

/// Adds the preshared key of SpiceDB to each call.
#[derive(Clone)]
pub(super) struct PresharedKey(MetadataValue<Ascii>);

impl Interceptor for PresharedKey {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        request
            .metadata_mut()
            .insert("authorization", self.0.clone());
        Ok(request)
    }
}

/// The channel to SpiceDB, with the key on each call.
type Authed = InterceptedService<Channel, PresharedKey>;

/// A client of SpiceDB.
#[derive(Clone)]
pub struct SpiceDb {
    /// The permissions API.
    pub(super) permissions: PermissionsServiceClient<Authed>,
    /// The schema API.
    schema: SchemaServiceClient<Authed>,
}

impl SpiceDb {
    /// Makes a client. It connects at the first call.
    pub fn new(endpoint: &str, key: &str) -> Result<Self, String> {
        // Make a channel that connects at the first call.
        let channel = Endpoint::from_shared(endpoint.to_owned())
            .map_err(|_| format!("{endpoint:?} is not a SpiceDB endpoint"))?
            .connect_lazy();

        // Send the preshared key with each call of the two APIs.
        let key = MetadataValue::try_from(format!("Bearer {key}"))
            .map_err(|_| "the SpiceDB key must be ASCII".to_owned())?;
        let interceptor = PresharedKey(key);
        Ok(Self {
            permissions: PermissionsServiceClient::with_interceptor(
                channel.clone(),
                interceptor.clone(),
            ),
            schema: SchemaServiceClient::with_interceptor(channel, interceptor),
        })
    }

    /// Writes the full schema.
    pub async fn write_schema(&self, schema: String) -> Result<(), AccessError> {
        self.schema
            .clone()
            .write_schema(WriteSchemaRequest { schema })
            .await
            .map_err(unavailable)?;
        Ok(())
    }

    /// Asks if `subject` has each permission on its object, in one call. An answer is `None`
    /// when SpiceDB could not check its item.
    pub(super) async fn check_bulk(
        &self,
        subject: &Object,
        items: &[(Object, &'static str)],
        fresh: Option<&str>,
    ) -> Result<Vec<Option<bool>>, AccessError> {
        // See at least the write of the check token, or else all the writes.
        let requirement = match fresh {
            Some(token) if !token.is_empty() => Requirement::AtLeastAsFresh(ZedToken {
                token: token.to_owned(),
            }),
            _ => Requirement::FullyConsistent(true),
        };

        // Ask for each permission on its object in one call.
        let request = CheckBulkPermissionsRequest {
            consistency: Some(Consistency {
                requirement: Some(requirement),
            }),
            items: items
                .iter()
                .map(|(object, permission)| CheckBulkPermissionsRequestItem {
                    resource: Some(reference(object)),
                    permission: (*permission).to_owned(),
                    subject: Some(subject_reference(subject)),
                    context: None,
                })
                .collect(),
            with_tracing: false,
        };
        let response = self
            .permissions
            .clone()
            .check_bulk_permissions(request)
            .await
            .map_err(unavailable)?;

        // Read one answer for each item, in the order of the items.
        let answers: Vec<_> = response
            .into_inner()
            .pairs
            .into_iter()
            .map(|pair| match pair.response {
                Some(Answer::Item(item)) => {
                    Some(item.permissionship == Permissionship::HasPermission as i32)
                }
                Some(Answer::Error(status)) => {
                    tracing::error!(message = status.message, "a SpiceDB check failed");
                    None
                }
                None => None,
            })
            .collect();
        if answers.len() != items.len() {
            return Err(AccessError::Unavailable(
                "permission system unavailable".to_owned(),
            ));
        }
        Ok(answers)
    }

    /// Returns the ids of the objects of a definition on which `subject` has `permission`.
    pub(super) async fn lookup(
        &self,
        subject: &Object,
        definition: &str,
        permission: &str,
    ) -> Result<Vec<String>, AccessError> {
        // Ask for all the objects, with all the writes seen.
        let request = LookupResourcesRequest {
            consistency: Some(Consistency {
                requirement: Some(Requirement::FullyConsistent(true)),
            }),
            resource_object_type: definition.to_owned(),
            permission: permission.to_owned(),
            subject: Some(subject_reference(subject)),
            ..Default::default()
        };
        let mut answers = self
            .permissions
            .clone()
            .lookup_resources(request)
            .await
            .map_err(unavailable)?
            .into_inner();

        // Keep the objects with a sure permission. A conditional one needs context that we never send.
        let mut ids = Vec::new();
        while let Some(answer) = answers.message().await.map_err(unavailable)? {
            if answer.permissionship == LookupPermissionship::HasPermission as i32 {
                ids.push(answer.resource_object_id);
            }
        }
        Ok(ids)
    }
}

/// Makes an object reference.
pub(super) fn reference(object: &Object) -> ObjectReference {
    ObjectReference {
        object_type: object.kind().to_owned(),
        object_id: object.id().to_owned(),
    }
}

/// Makes a subject reference.
pub(super) fn subject_reference(subject: &Object) -> SubjectReference {
    SubjectReference {
        object: Some(reference(subject)),
        optional_relation: String::new(),
    }
}

/// Logs a SpiceDB failure and hides its details from the client.
pub(super) fn unavailable(status: Status) -> AccessError {
    tracing::error!(%status, "SpiceDB call failed");
    AccessError::Unavailable("permission system unavailable".to_owned())
}
