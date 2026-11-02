use scylla_domain::users::UserId;
use scylla_domain::ResourceKind;

#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct CreateContext {
    pub kind: ResourceKind,
    pub owner_id: UserId,
}

impl CreateContext {
    pub fn new(kind: ResourceKind, owner_id: UserId) -> Self {
        Self { kind, owner_id }
    }
}

#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct DeleteContext {
    pub kind: ResourceKind,
    pub owner_id: UserId,
}

impl DeleteContext {
    pub fn new(kind: ResourceKind, owner_id: UserId) -> Self {
        Self { kind, owner_id }
    }
}
