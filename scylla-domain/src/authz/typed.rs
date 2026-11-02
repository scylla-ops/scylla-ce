//! The typed names of the model: a resource, its relations and its permissions.

use std::fmt;
use std::marker::PhantomData;

use super::{Check, Object, Relationship};
use crate::Id;

/// A type of object in the permission model: one SpiceDB definition. `authz!` implements it.
pub trait Resource: 'static {
    /// The name of the definition, for example `project`.
    const NAME: &'static str;
}

/// A permission of the resource `R`, for example `Project::DELETE`.
pub struct Permission<R> {
    /// The name in the schema.
    name: &'static str,
    /// The resource, as a marker only.
    resource: PhantomData<fn() -> R>,
}

impl<R: Resource> Permission<R> {
    /// Makes a permission. Only `authz!` calls it, so each permission is in the schema.
    #[doc(hidden)]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            resource: PhantomData,
        }
    }

    /// Returns the name in the schema.
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// Returns the check of this permission on one object: `Project::DELETE.on(id)`.
    pub fn on(self, id: Id<R>) -> Check {
        Check::new(Object::of(id), self.name)
    }
}

impl<R> Clone for Permission<R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R> Copy for Permission<R> {}

impl<R: Resource> fmt::Debug for Permission<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", R::NAME, self.name)
    }
}

/// A relation of the resource `R` to subjects of the resource `S`, for example `Project::OWNER`.
pub struct Relation<R, S> {
    /// The name in the schema.
    name: &'static str,
    /// The resource and the subject, as markers only.
    types: PhantomData<fn() -> (R, S)>,
}

impl<R: Resource, S: Resource> Relation<R, S> {
    /// Makes a relation. Only `authz!` calls it, so each relation is in the schema.
    #[doc(hidden)]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            types: PhantomData,
        }
    }

    /// Returns the name in the schema.
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// Returns the relationship `object#relation@subject`: `Project::OWNER.of(project, owner)`.
    pub fn of(self, object: Id<R>, subject: Id<S>) -> Relationship {
        Relationship::new(Object::of(object), self.name, Object::of(subject))
    }
}

impl<R, S> Clone for Relation<R, S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R, S> Copy for Relation<R, S> {}

impl<R: Resource, S: Resource> fmt::Debug for Relation<R, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}@{}", R::NAME, self.name, S::NAME)
    }
}
