//! The relationship calls of the SpiceDB client: writes, deletes, and the roles on an object.

use scylla_domain::authz::{self, Object, Resource};
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::PageSize;
use scylla_extension::{AccessError, RoleHolder, RolePage};
use spicedb_grpc_tonic::v1::consistency::Requirement;
use spicedb_grpc_tonic::v1::relationship_update::Operation;
use spicedb_grpc_tonic::v1::{
    Consistency, Cursor, DeleteRelationshipsRequest, ReadRelationshipsRequest, Relationship,
    RelationshipFilter, RelationshipUpdate, SubjectFilter, WriteRelationshipsRequest,
};
use tonic::{Code, Status};

use super::spicedb::{reference, subject_reference, unavailable};
use super::SpiceDb;

/// The limit of each read of the roles: one more than the largest page.
const READ_LIMIT: u32 = PageSize::MAX as u32 + 1;

impl SpiceDb {
    /// Applies one operation to relationships, and returns the token of the write.
    pub(super) async fn update(
        &self,
        relationships: &[authz::Relationship],
        operation: Operation,
    ) -> Result<String, AccessError> {
        // Make one update for each relationship.
        let updates = relationships
            .iter()
            .map(|relationship| RelationshipUpdate {
                operation: operation as i32,
                relationship: Some(Relationship {
                    resource: Some(reference(relationship.object())),
                    relation: relationship.relation().to_owned(),
                    subject: Some(subject_reference(relationship.subject())),
                    ..Default::default()
                }),
            })
            .collect();

        // Write them in one call, and keep the token of the write.
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

    /// Deletes all relationships of an object.
    pub(super) async fn delete_object(&self, object: &Object) -> Result<(), AccessError> {
        let request = DeleteRelationshipsRequest {
            relationship_filter: Some(RelationshipFilter {
                resource_type: object.kind().to_owned(),
                optional_resource_id: object.id().to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        };
        self.delete(request).await
    }

    /// Deletes all relationships of a subject, on each definition.
    pub(super) async fn delete_subject(&self, subject: &Object) -> Result<(), AccessError> {
        let request = DeleteRelationshipsRequest {
            relationship_filter: Some(RelationshipFilter {
                optional_subject_filter: Some(SubjectFilter {
                    subject_type: subject.kind().to_owned(),
                    optional_subject_id: subject.id().to_owned(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        self.delete(request).await
    }

    /// Returns the ids of the subjects of the relationships of an object.
    pub(super) async fn subjects(&self, object: &Object) -> Result<Vec<String>, AccessError> {
        // Read each relationship of the object, with all the writes seen.
        let request = ReadRelationshipsRequest {
            consistency: Some(Consistency {
                requirement: Some(Requirement::FullyConsistent(true)),
            }),
            relationship_filter: Some(RelationshipFilter {
                resource_type: object.kind().to_owned(),
                optional_resource_id: object.id().to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut answers = self
            .permissions
            .clone()
            .read_relationships(request)
            .await
            .map_err(unavailable)?
            .into_inner();

        // Keep the id of each subject.
        let mut ids = Vec::new();
        while let Some(answer) = answers.message().await.map_err(unavailable)? {
            let subject = answer.relationship.and_then(|found| found.subject);
            if let Some(reference) = subject.and_then(|subject| subject.object) {
                ids.push(reference.object_id);
            }
        }
        Ok(ids)
    }

    /// Reads up to `count` roles of actors on an object, after a cursor or from the first role.
    /// Returns `None` if SpiceDB refuses the cursor.
    pub(super) async fn roles(
        &self,
        object: &Object,
        cursor: Option<&str>,
        count: usize,
    ) -> Result<Option<RolePage>, AccessError> {
        // Read the roles with a fixed limit, as a cursor needs the same request each time.
        let request = ReadRelationshipsRequest {
            consistency: Some(Consistency {
                requirement: Some(Requirement::FullyConsistent(true)),
            }),
            relationship_filter: Some(RelationshipFilter {
                resource_type: object.kind().to_owned(),
                optional_resource_id: object.id().to_owned(),
                optional_subject_filter: Some(SubjectFilter {
                    subject_type: Actor::NAME.to_owned(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            optional_limit: READ_LIMIT,
            optional_cursor: cursor.map(|token| Cursor {
                token: token.to_owned(),
            }),
        };
        let refused = |status: Status| match status.code() {
            Code::InvalidArgument | Code::FailedPrecondition if cursor.is_some() => Ok(None),
            _ => Err(unavailable(status)),
        };
        let mut answers = match self.permissions.clone().read_relationships(request).await {
            Ok(response) => response.into_inner(),
            Err(status) => return refused(status),
        };

        // Keep `count` roles, and look for one more.
        let mut page = RolePage::default();
        loop {
            let answer = match answers.message().await {
                Ok(Some(answer)) => answer,
                Ok(None) => break,
                Err(status) => return refused(status),
            };
            if page.holders.len() == count {
                page.more = true;
                break;
            }
            let Some(relationship) = answer.relationship else {
                continue;
            };
            let subject = relationship.subject.and_then(|subject| subject.object);
            if let Some(actor) = subject.and_then(|object| ActorId::parse(&object.object_id).ok()) {
                page.holders.push(RoleHolder {
                    role: relationship.relation,
                    actor,
                });
                page.cursor = answer.after_result_cursor.map(|cursor| cursor.token);
            }
        }
        Ok(Some(page))
    }

    /// Sends a delete of relationships.
    async fn delete(&self, request: DeleteRelationshipsRequest) -> Result<(), AccessError> {
        self.permissions
            .clone()
            .delete_relationships(request)
            .await
            .map_err(unavailable)?;
        Ok(())
    }
}
