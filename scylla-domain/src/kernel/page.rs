//! Keyset pagination: a page starts after the last key of the previous page, an id or a unique
//! name.

use crate::DomainError;

/// The number of items in one page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageSize(usize);

impl PageSize {
    /// The size when the client gives 0.
    pub const DEFAULT: usize = 50;
    /// The largest size. A larger request gets this size.
    pub const MAX: usize = 100;

    /// Reads a size: 0 gives the default, a value above the maximum gives the maximum.
    pub fn parse(raw: i32) -> Result<Self, DomainError> {
        match usize::try_from(raw) {
            Ok(0) => Ok(Self::default()),
            Ok(size) => Ok(Self(size.min(Self::MAX))),
            Err(_) => Err(DomainError::Negative),
        }
    }

    /// Returns the size as a number of items.
    pub fn get(self) -> usize {
        self.0
    }
}

impl Default for PageSize {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

/// The page that a client asks for. `C` is the key that sorts the items: an id, or a unique
/// name.
#[derive(Clone, Copy, Debug)]
pub struct PageRequest<C> {
    /// Gives only the items after this key. `None` gives the first page.
    pub after: Option<C>,
    /// The maximum number of items.
    pub size: PageSize,
}

impl<C> Default for PageRequest<C> {
    fn default() -> Self {
        Self {
            after: None,
            size: PageSize::default(),
        }
    }
}

/// One page of items, and the key to give to get the next page.
#[derive(Clone, Debug)]
pub struct Page<T, C> {
    /// The items, in key order.
    pub items: Vec<T>,
    /// The key of the last item, if a next page exists.
    pub next_after: Option<C>,
}

impl<T, C> Page<T, C> {
    /// Makes a page from a store result that reads one item more than the page size.
    pub fn from_overfetch(mut items: Vec<T>, size: PageSize, id: impl Fn(&T) -> C) -> Self {
        let next_after = if items.len() > size.get() {
            items.truncate(size.get());
            items.last().map(id)
        } else {
            None
        };
        Self { items, next_after }
    }
}
