//! The typed id of an entity.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use uuid::Uuid;

use super::codec;
use crate::DomainError;

/// The id of an entity of type `T`: 128 opaque bits, ordered by creation time.
pub struct Id<T: ?Sized> {
    /// The bits of the id.
    bits: u128,
    /// The entity type, as a marker only.
    kind: PhantomData<fn() -> T>,
}

impl<T: ?Sized> Id<T> {
    /// Makes a new id.
    pub fn generate() -> Self {
        Self::from_bits(codec::generate())
    }

    /// Reads an id from its text form.
    pub fn parse(text: &str) -> Result<Self, DomainError> {
        codec::decode(text)
            .map(Self::from_bits)
            .ok_or(DomainError::NotAnId)
    }

    /// Makes an id from the value of a Postgres `uuid` column.
    pub fn from_db(value: Uuid) -> Self {
        Self::from_bits(value.as_u128())
    }

    /// Returns the value for a Postgres `uuid` column.
    pub fn to_db(self) -> Uuid {
        Uuid::from_u128(self.bits)
    }

    /// Returns the 128 bits in big-endian order. They do not change with the text form, so they
    /// can bind a value to its id, for example as the associated data of an encryption.
    pub fn to_bytes(self) -> [u8; 16] {
        self.bits.to_be_bytes()
    }

    /// Returns the id of the only object of a type: each bit is zero.
    pub const fn singleton() -> Self {
        Self::from_bits(0)
    }

    /// Returns the same bits as the id of another type, for an entity that is also an actor.
    pub(crate) const fn retype<U: ?Sized>(self) -> Id<U> {
        Id::from_bits(self.bits)
    }

    /// Makes an id from its bits.
    const fn from_bits(bits: u128) -> Self {
        Self {
            bits,
            kind: PhantomData,
        }
    }
}

impl<T: ?Sized> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> Copy for Id<T> {}

impl<T: ?Sized> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.bits == other.bits
    }
}

impl<T: ?Sized> Eq for Id<T> {}

impl<T: ?Sized> Hash for Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bits.hash(state);
    }
}

impl<T: ?Sized> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({self})")
    }
}

impl<T: ?Sized> fmt::Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&codec::encode(self.bits))
    }
}
