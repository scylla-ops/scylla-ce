use crate::authz::Access;

/// What every action has: the access rule the authorize stage applies.
pub trait Describe: Send + Sync + 'static {
    fn access(&self) -> Access;
}

/// A write. The two payload types say whether the thing is in the store: `Draft<T>` is not,
/// `T` is, `Deleted<T>` was.
pub trait Command: Describe {
    type Staged: Send + Sync + 'static;
    type Committed: Send + Sync + 'static;
}

/// A read. It has no staged value and no write, so it takes two stages instead of three.
pub trait Query: Describe {
    type Output: Send + Sync + 'static;
}
