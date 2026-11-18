//! One page of the open items of a workspace, newest first, with the counts of all the pages.

use std::cmp::Ordering;

use time::OffsetDateTime;

use super::{AttentionCursor, AttentionItem, AttentionKind, AttentionQuery};

/// The open items of one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KindCount {
    /// The kind.
    pub kind: AttentionKind,
    /// The open items of the kind.
    pub open: u32,
    /// The open items of the kind after the last visit.
    pub new: u32,
}

/// One page of open items, and the counts of all the pages.
#[derive(Clone, Debug, PartialEq)]
pub struct AttentionList {
    /// The items of the page, newest first.
    pub items: Vec<AttentionItem>,
    /// The counts of each kind that the query asks for and that the caller reads.
    pub counts: Vec<KindCount>,
    /// The open items of all the kinds.
    pub open: u32,
    /// The open items of all the kinds after the last visit.
    pub new: u32,
    /// The key of the last item, if a next page exists.
    pub next_after: Option<AttentionCursor>,
    /// The last time when an open item of a job or of a fire went away.
    pub last_cleared_at: Option<OffsetDateTime>,
}

impl AttentionList {
    /// Makes the page of a query from all its open items: marks the new items, counts them by
    /// kind, sorts them newest first and keeps the items after the key of the page.
    pub fn page(
        mut items: Vec<AttentionItem>,
        query: &AttentionQuery,
        last_cleared_at: Option<OffsetDateTime>,
    ) -> Self {
        // Mark the new items, and count the items of each kind.
        for item in &mut items {
            item.is_new = query.since.is_some_and(|since| item.at > since);
        }
        let counts: Vec<KindCount> = query
            .open_kinds()
            .into_iter()
            .map(|kind| {
                let of_kind = items.iter().filter(|item| item.kind == kind);
                KindCount {
                    kind,
                    open: total(of_kind.clone().count()),
                    new: total(of_kind.filter(|item| item.is_new).count()),
                }
            })
            .collect();

        // Sort the items, then keep one page after the key.
        items.sort_by(newest_first);
        if let Some(after) = query.page.after {
            items.retain(|item| is_after(item, &after));
        }
        let size = query.page.size.get();
        let next_after = (items.len() > size).then(|| AttentionCursor {
            at: items[size - 1].at,
            id: items[size - 1].id(),
        });
        items.truncate(size);
        let sum = |part: fn(&KindCount) -> u32| {
            counts
                .iter()
                .fold(0, |sum: u32, count| sum.saturating_add(part(count)))
        };
        Self {
            open: sum(|count| count.open),
            new: sum(|count| count.new),
            items,
            counts,
            next_after,
            last_cleared_at,
        }
    }
}

/// Orders two items: the newest time first, then the largest id.
fn newest_first(a: &AttentionItem, b: &AttentionItem) -> Ordering {
    b.at.cmp(&a.at)
        .then_with(|| b.id().to_bytes().cmp(&a.id().to_bytes()))
}

/// Returns `true` if an item comes after the key of a page.
fn is_after(item: &AttentionItem, after: &AttentionCursor) -> bool {
    let key = (after.at, after.id.to_bytes());
    (item.at, item.id().to_bytes()) < key
}

/// Returns a count as a number of the answer.
fn total(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}
