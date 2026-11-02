//! The SpiceDB client: permission checks, relation writes and the schema.

use scylla_extension::{AccessError, Object, Relation};
use spicedb_grpc_tonic::v1::consistency::Requirement;
use spicedb_grpc_tonic::v1::permissions_service_client::PermissionsServiceClient;
use spicedb_grpc_tonic::v1::relationship_update::Operation;
use spicedb_grpc_tonic::v1::schema_service_client::SchemaServiceClient;
use spicedb_grpc_tonic::v1::{
    check_permission_response::Permissionship, CheckPermissionRequest, Consistency,
    DeleteRelationshipsRequest, ObjectReference, Relationship, RelationshipFilter,
    RelationshipUpdate, SubjectReference, WriteRelationshipsRequest, WriteSchemaRequest, ZedToken,
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
    permissions: PermissionsServiceClient<Authed>,
    /// The schema API.
    schema: SchemaServiceClient<Authed>,
}

impl SpiceDb {
    /// Makes a client. It connects at the first call.
    pub fn new(endpoint: &str, key: &str) -> Result<Self, String> {
        let channel = Endpoint::from_shared(endpoint.to_owned())
            .map_err(|_| format!("{endpoint:?} is not a SpiceDB endpoint"))?
            .connect_lazy();
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

    /// Returns `true` if `subject` has `permission` on `object`.
    pub(super) async fn check(
        &self,
        subject: &Object,
        object: &Object,
        permission: &str,
        fresh: Option<&str>,
    ) -> Result<bool, AccessError> {
        let requirement = match fresh {
            Some(token) if !token.is_empty() => Requirement::AtLeastAsFresh(ZedToken {
                token: token.to_owned(),
            }),
            _ => Requirement::FullyConsistent(true),
        };
        let request = CheckPermissionRequest {
            consistency: Some(Consistency {
                requirement: Some(requirement),
            }),
            resource: Some(reference(object)),
            permission: permission.to_owned(),
            subject: Some(subject_reference(subject)),
            ..Default::default()
        };
        let response = self
            .permissions
            .clone()
            .check_permission(request)
            .await
            .map_err(unavailable)?;
        Ok(response.into_inner().permissionship == Permissionship::HasPermission as i32)
    }

    /// Applies one operation to relations, and returns the token of the write.
    pub(super) async fn update(
        &self,
        relations: &[Relation],
        operation: Operation,
    ) -> Result<String, AccessError> {
        let updates = relations
            .iter()
            .map(|relation| RelationshipUpdate {
                operation: operation as i32,
                relationship: Some(Relationship {
                    resource: Some(reference(&relation.object)),
                    relation: relation.relation.to_owned(),
                    subject: Some(subject_reference(&relation.subject)),
                    ..Default::default()
                }),
            })
            .collect();
        let request = WriteRelationshipsRequest {
            updates,
            ..Default::default()
        };
        let response = self
            .permissions
            .clone()
            .write_relationships(request)
            .await
            .map_err(unavailable)?;
        Ok(response
            .into_inner()
            .written_at
            .map(|token| token.token)
            .unwrap_or_default())
    }

    /// Deletes all relations of an object.
    pub(super) async fn delete_object(&self, object: &Object) -> Result<(), AccessError> {
        let request = DeleteRelationshipsRequest {
            relationship_filter: Some(RelationshipFilter {
                resource_type: object.kind.to_owned(),
                optional_resource_id: object.id.clone(),
                ..Default::default()
            }),
            ..Default::default()
        };
        self.permissions
            .clone()
            .delete_relationships(request)
            .await
            .map_err(unavailable)?;
        Ok(())
    }
}

/// Makes an object reference.
fn reference(object: &Object) -> ObjectReference {
    ObjectReference {
        object_type: object.kind.to_owned(),
        object_id: object.id.clone(),
    }
}

/// Makes a subject reference.
fn subject_reference(subject: &Object) -> SubjectReference {
    SubjectReference {
        object: Some(reference(subject)),
        optional_relation: String::new(),
    }
}

/// Logs a SpiceDB failure and hides its details from the client.
fn unavailable(status: Status) -> AccessError {
    tracing::error!(%status, "SpiceDB call failed");
    AccessError::Unavailable("permission system unavailable".to_owned())
}
