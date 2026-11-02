//! The edition of the `scylla-ce` binary.

use scylla_extension::Edition;

/// Makes the Community edition: the core plugins only.
pub fn edition() -> Edition {
    scylla_core::edition("community").build()
}
