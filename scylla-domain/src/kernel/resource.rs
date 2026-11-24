//! The kind of a resource: the key of the hooks.

/// The kind of a resource, for example `"project"`. Each feature exports its own as `KIND`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceKind(&'static str);

impl ResourceKind {
    /// Makes a kind from its name, singular and in snake case.
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// Returns the name of the kind.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}
