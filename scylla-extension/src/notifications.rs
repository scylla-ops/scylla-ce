//! The notification kinds of an edition.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::{Audience, Params};

/// What a reader needs to see a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// The row is about the reader alone.
    Own,
    /// The reader needs this route permission on the project, the workspace or the platform of
    /// the row. A row with the reason `REASON_SUBJECT` needs nothing.
    Holds(&'static str),
}

/// What a second fact does to the open row of its group.
#[derive(Clone, Copy, Debug)]
pub enum Collapse {
    /// The fact adds 1 to the count, moves the row up, and makes it unread.
    Count,
    /// The fact replaces the parameters.
    Replace,
    /// The fact changes nothing.
    Keep,
    /// The fact merges with a row that is younger than the window, else it makes a new row.
    Merge {
        /// The age of the row after which a fact makes a new row.
        window: Duration,
        /// Adds the parameters of the fact to those of the row.
        merge: fn(&mut Params, &Params),
    },
}

/// A kind of notification.
#[derive(Clone)]
#[non_exhaustive]
pub struct NotificationKind {
    /// The key, as `<subject>.<action>`. The key never changes.
    pub key: &'static str,
    /// The category, for the filters of the inbox.
    pub category: &'static str,
    /// What a reader needs to see a row.
    pub access: Access,
    /// Who gets a row.
    pub audience: Arc<dyn Audience>,
    /// What a second fact of a group does.
    pub collapse: Collapse,
    /// `true` if a row waits for an action of the reader.
    pub needs_action: bool,
    /// `true` if the person that the fact is about always gets the row, also for an own action.
    pub mandatory: bool,
}

impl NotificationKind {
    /// Makes a kind about the reader alone, with no grouping.
    pub fn new(key: &'static str, category: &'static str, audience: Arc<dyn Audience>) -> Self {
        Self {
            key,
            category,
            access: Access::Own,
            audience,
            collapse: Collapse::Keep,
            needs_action: false,
            mandatory: false,
        }
    }

    /// Sets what a reader needs to see a row.
    pub fn access(mut self, access: Access) -> Self {
        self.access = access;
        self
    }

    /// Sets what a second fact of a group does.
    pub fn collapse(mut self, collapse: Collapse) -> Self {
        self.collapse = collapse;
        self
    }

    /// Makes each row wait for an action of the reader.
    pub fn needs_action(mut self) -> Self {
        self.needs_action = true;
        self
    }

    /// Makes the row reach the person that the fact is about, also for an own action.
    pub fn mandatory(mut self) -> Self {
        self.mandatory = true;
        self
    }
}

/// The kinds of an edition.
#[derive(Default)]
pub struct NotificationKinds {
    /// The kinds, by key.
    kinds: HashMap<&'static str, NotificationKind>,
}

impl NotificationKinds {
    /// Returns a kind.
    pub fn get(&self, key: &str) -> Option<&NotificationKind> {
        self.kinds.get(key)
    }

    /// Returns each kind.
    pub fn all(&self) -> impl Iterator<Item = &NotificationKind> {
        self.kinds.values()
    }

    /// Returns the keys of the kinds of a category.
    pub fn keys_of(&self, category: &str) -> Vec<String> {
        self.all()
            .filter(|kind| kind.category == category)
            .map(|kind| kind.key.to_owned())
            .collect()
    }

    /// Adds a kind. A key that is not `<subject>.<action>`, or that has a kind, gives `false`.
    pub(crate) fn add(&mut self, kind: NotificationKind) -> bool {
        if !valid_key(kind.key) || self.kinds.contains_key(kind.key) {
            return false;
        }
        self.kinds.insert(kind.key, kind);
        true
    }
}

/// Returns `true` for a key `<subject>.<action>`, in lower snake case.
fn valid_key(key: &str) -> bool {
    let word = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|letter| letter.is_ascii_lowercase() || letter == '_')
    };
    matches!(key.split_once('.'), Some((subject, action)) if word(subject) && word(action))
}
