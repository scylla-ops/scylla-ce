//! Unit tests of the order of the groups and of the projects.

use super::{place, place_at};

/// A place before another item puts the item just before it, out of its old place.
#[test]
fn place_puts_the_item_before_the_other_item() {
    // given
    let mut order = vec![1, 2, 3, 4];

    // when
    place(&mut order, 4, Some(2));

    // then
    assert_eq!(order, vec![1, 4, 2, 3]);
}

/// No other item, or an other item that is not in the order, puts the item last.
#[test]
fn place_puts_the_item_last_with_no_known_other_item() {
    // given
    let mut order = vec![1, 2, 3];
    let mut unknown = vec![1, 2, 3];

    // when
    place(&mut order, 1, None);
    place(&mut unknown, 5, Some(9));

    // then
    assert_eq!(order, vec![2, 3, 1]);
    assert_eq!(unknown, vec![1, 2, 3, 5]);
}

/// A new item goes into the order at its place.
#[test]
fn place_adds_a_new_item() {
    // given
    let mut order = vec![1, 2];

    // when
    place(&mut order, 7, Some(2));

    // then
    assert_eq!(order, vec![1, 7, 2]);
}

/// An index puts the item at that index after it leaves its old place, and an index after the
/// end puts it last.
#[test]
fn place_at_moves_the_item_to_the_index() {
    // given
    let mut up = vec![1, 2, 3, 4];
    let mut past = vec![1, 2, 3];

    // when
    place_at(&mut up, 3, 0);
    place_at(&mut past, 1, 10);

    // then
    assert_eq!(up, vec![3, 1, 2, 4]);
    assert_eq!(past, vec![2, 3, 1]);
}
