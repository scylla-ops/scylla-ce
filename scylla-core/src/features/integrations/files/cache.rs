//! The memory of the pipeline files: the bytes of the blobs that the server read, by project and
//! blob SHA, the files of the folders at each commit that the server read, and the last check of
//! the head of the default branch of each project. The server is one instance, so a restart only
//! reads the files again.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use scylla_domain::projects::ProjectId;

use super::repo::Entry;

/// The most bytes of blobs that the memory keeps. The oldest blobs go first.
const MAX_BYTES: usize = 32 << 20;

/// The most commits whose folders the memory keeps. The oldest commits go first.
const MAX_TREES: usize = 1024;

/// The blobs that the server read.
static BLOBS: LazyLock<Mutex<Blobs>> = LazyLock::new(Mutex::default);

/// The files of the folders at each commit that the server read.
static TREES: LazyLock<Mutex<Trees>> = LazyLock::new(Mutex::default);

/// The last check of the head of the default branch of each project.
static HEADS: LazyLock<Mutex<HashMap<ProjectId, Seen>>> = LazyLock::new(Mutex::default);

/// The bytes of the blobs, in the order of their reads.
#[derive(Default)]
struct Blobs {
    /// The bytes of each project and blob SHA.
    bytes: HashMap<(ProjectId, String), Arc<[u8]>>,
    /// The keys, oldest first.
    order: VecDeque<(ProjectId, String)>,
    /// The sum of the bytes.
    total: usize,
}

/// The files of the folders at the commits, in the order of their reads.
#[derive(Default)]
struct Trees {
    /// The files of each project and commit.
    entries: HashMap<(ProjectId, String), Arc<[Entry]>>,
    /// The keys, oldest first.
    order: VecDeque<(ProjectId, String)>,
}

/// The last check of the head of the default branch of a project.
#[derive(Default)]
struct Seen {
    /// The time of the last check.
    checked_at: Option<Instant>,
    /// The head that the last sync read.
    head: Option<String>,
}

/// Returns the bytes of a blob that the server read for a project.
pub(super) fn blob(project: ProjectId, sha: &str) -> Option<Arc<[u8]>> {
    let blobs = BLOBS.lock().unwrap_or_else(PoisonError::into_inner);
    blobs.bytes.get(&(project, sha.to_owned())).cloned()
}

/// Keeps the bytes of a blob of a project, and forgets the oldest blobs over the limit.
pub(super) fn keep(project: ProjectId, sha: &str, bytes: Arc<[u8]>) {
    let mut blobs = BLOBS.lock().unwrap_or_else(PoisonError::into_inner);
    let key = (project, sha.to_owned());
    if bytes.len() > MAX_BYTES || blobs.bytes.contains_key(&key) {
        return;
    }
    blobs.total += bytes.len();
    blobs.order.push_back(key.clone());
    blobs.bytes.insert(key, bytes);
    while blobs.total > MAX_BYTES {
        let Some(oldest) = blobs.order.pop_front() else {
            break;
        };
        if let Some(gone) = blobs.bytes.remove(&oldest) {
            blobs.total -= gone.len();
        }
    }
}

/// Returns the files of the folders of a project at a commit that the server read.
pub(super) fn tree(project: ProjectId, commit: &str) -> Option<Arc<[Entry]>> {
    let trees = TREES.lock().unwrap_or_else(PoisonError::into_inner);
    trees.entries.get(&(project, commit.to_owned())).cloned()
}

/// Keeps the files of the folders of a project at a commit, and forgets the oldest commits over
/// the limit.
pub(super) fn keep_tree(project: ProjectId, commit: &str, entries: Arc<[Entry]>) {
    let mut trees = TREES.lock().unwrap_or_else(PoisonError::into_inner);
    let key = (project, commit.to_owned());
    if trees.entries.insert(key.clone(), entries).is_none() {
        trees.order.push_back(key);
    }
    while trees.order.len() > MAX_TREES {
        if let Some(oldest) = trees.order.pop_front() {
            trees.entries.remove(&oldest);
        }
    }
}

/// Returns `true`, and records the check, when the last check of a project is older than
/// `every`.
pub(super) fn due(project: ProjectId, every: Duration) -> bool {
    let mut heads = HEADS.lock().unwrap_or_else(PoisonError::into_inner);
    let seen = heads.entry(project).or_default();
    let now = Instant::now();
    if seen
        .checked_at
        .is_some_and(|at| now.duration_since(at) < every)
    {
        return false;
    }
    seen.checked_at = Some(now);
    true
}

/// Returns the head of the default branch that the last sync of a project read.
pub(super) fn head(project: ProjectId) -> Option<String> {
    let heads = HEADS.lock().unwrap_or_else(PoisonError::into_inner);
    heads.get(&project).and_then(|seen| seen.head.clone())
}

/// Records the head of the default branch that a sync of a project read, as a check of now.
pub(super) fn saw(project: ProjectId, head: &str) {
    let mut heads = HEADS.lock().unwrap_or_else(PoisonError::into_inner);
    let seen = heads.entry(project).or_default();
    seen.checked_at = Some(Instant::now());
    seen.head = Some(head.to_owned());
}
