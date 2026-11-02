//! The format of the ids: the only file that knows it. Today UUIDv7.

use uuid::Uuid;

/// Makes the bits of a new id, ordered by creation time.
pub(crate) fn generate() -> u128 {
    Uuid::now_v7().as_u128()
}

/// Writes the bits of an id as text.
pub(crate) fn encode(bits: u128) -> String {
    Uuid::from_u128(bits).to_string()
}

/// Reads the bits of an id from text, or `None` if the text is not an id.
pub(crate) fn decode(text: &str) -> Option<u128> {
    Uuid::parse_str(text).ok().map(|uuid| uuid.as_u128())
}
