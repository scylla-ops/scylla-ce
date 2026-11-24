//! The ids of the old product: ULIDs in lower case. A ULID has 128 bits, as an id of this base,
//! so an old id keeps its bits.

use uuid::Uuid;

/// The alphabet of the Crockford base 32 of the ULIDs.
const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// Reads an old id: a ULID of 26 characters. Returns `None` for another text.
pub(super) fn of_ulid(text: &str) -> Option<Uuid> {
    let text = text.to_ascii_lowercase();
    if text.len() != 26 {
        return None;
    }
    let mut bits: u128 = 0;
    for (index, byte) in text.bytes().enumerate() {
        let value = ALPHABET.iter().position(|letter| *letter == byte)? as u128;
        if index == 0 && value > 7 {
            return None;
        }
        bits = (bits << 5) | value;
    }
    Some(Uuid::from_u128(bits))
}

#[cfg(test)]
mod tests {
    //! Unit tests of the old ids.

    use super::of_ulid;

    /// A ULID keeps its bits, and another text is refused.
    #[test]
    fn a_ulid_gives_its_bits() {
        // given
        let ulid = "01arz3ndektsv4rrffq69g5fav";

        // when
        let id = of_ulid(ulid).unwrap();

        // then
        assert_eq!(id.to_string(), "01563e3a-b5d3-d676-4c61-efb99302bd5b");
        assert!(of_ulid("not a ulid").is_none());
        assert!(of_ulid("81arz3ndektsv4rrffq69g5fav").is_none());
    }
}
