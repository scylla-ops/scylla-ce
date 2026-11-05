//! A gate for the unit tests of the projects service.

use std::collections::BTreeMap;

use scylla_domain::authz::{Check, Object, Relationship};
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{
    async_trait, AccessError, Binding, Gate, IssuedToken, ObjectRole, Operation, Principal,
    RolePage,
};
use tonic::metadata::MetadataMap;

/// A gate that allows each check and keeps no relationship: these tests check only the hooks.
pub(super) struct NoGate;

#[async_trait]
impl Gate for NoGate {
    async fn authenticate(
        &self,
        _metadata: &MetadataMap,
        _operation: &Operation,
    ) -> Result<Principal, AccessError> {
        Err(AccessError::Unauthenticated(
            "no gate in unit tests".to_owned(),
        ))
    }

    async fn authorize(&self, _principal: &Principal, _check: Check) -> Result<(), AccessError> {
        Ok(())
    }

    fn issue(&self, _actor: &Actor, _binding: Binding) -> Result<IssuedToken, AccessError> {
        Err(AccessError::Unavailable("no gate in unit tests".to_owned()))
    }

    async fn write(&self, _relationships: &[Relationship]) -> Result<String, AccessError> {
        Ok(String::new())
    }

    async fn delete(&self, _relationships: &[Relationship]) -> Result<(), AccessError> {
        Ok(())
    }

    async fn delete_object(&self, _object: &Object) -> Result<(), AccessError> {
        Ok(())
    }

    async fn delete_subject(&self, _subject: &Object) -> Result<(), AccessError> {
        Ok(())
    }

    async fn subjects(&self, _object: &Object) -> Result<Vec<ActorId>, AccessError> {
        Ok(Vec::new())
    }

    async fn roles(
        &self,
        _object: &Object,
        _cursor: Option<&str>,
        _count: usize,
    ) -> Result<Option<RolePage>, AccessError> {
        Ok(Some(RolePage::default()))
    }

    async fn permissions(
        &self,
        _principal: &Principal,
        _object: &Object,
        _workspace: Option<WorkspaceId>,
        permissions: &[&'static str],
    ) -> Result<Vec<bool>, AccessError> {
        Ok(vec![true; permissions.len()])
    }

    async fn roles_of(
        &self,
        _actor: ActorId,
        _definition: &'static str,
    ) -> Result<Vec<ObjectRole>, AccessError> {
        Ok(Vec::new())
    }

    async fn holders_of(
        &self,
        _definition: &'static str,
        _relation: &str,
    ) -> Result<Vec<ObjectRole>, AccessError> {
        Ok(Vec::new())
    }

    async fn holds(
        &self,
        _principal: &Principal,
        _object: &Object,
        _workspace: Option<WorkspaceId>,
        permissions: &[&'static str],
    ) -> Result<Vec<bool>, AccessError> {
        Ok(vec![true; permissions.len()])
    }

    async fn lookup(
        &self,
        _actor: ActorId,
        _definition: &'static str,
        _permission: &'static str,
    ) -> Result<Vec<String>, AccessError> {
        Ok(Vec::new())
    }

    async fn role_counts(
        &self,
        _definition: &'static str,
    ) -> Result<BTreeMap<String, u32>, AccessError> {
        Ok(BTreeMap::new())
    }
}
