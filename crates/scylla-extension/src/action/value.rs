use std::fmt;
use std::ops::Deref;

/// A value that is not in the store yet. `Persist` takes it out and writes it.
pub struct Draft<T>(T);

impl<T> Draft<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Draft<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: fmt::Debug> fmt::Debug for Draft<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Draft").field(&self.0).finish()
    }
}

/// A tombstone: the last state of a value the store no longer holds.
pub struct Deleted<T>(T);

impl<T> Deleted<T> {
    pub fn new(last_state: T) -> Self {
        Self(last_state)
    }

    pub fn last_state(&self) -> &T {
        &self.0
    }
}

impl<T: fmt::Debug> fmt::Debug for Deleted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Deleted").field(&self.0).finish()
    }
}
