//! A rate limit in memory: a number of requests for each key in each window. A public route
//! with no token uses it, with the IP address of the client or a value of the request as the key.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The number of keys above which a count forgets the keys of the windows that ended.
const MAX_KEYS: usize = 10_000;

/// The requests of each key in the current window.
pub(crate) struct Rates<K> {
    /// The most requests of one key in one window.
    rate: u32,
    /// The length of a window.
    window: Duration,
    /// The start of the window and the count of each key.
    counts: Mutex<HashMap<K, (Instant, u32)>>,
}

impl<K: Eq + Hash> Rates<K> {
    /// Makes a limit of `rate` requests of one key in each `window`.
    pub(crate) fn new(rate: u32, window: Duration) -> Self {
        Self {
            rate,
            window,
            counts: Mutex::default(),
        }
    }

    /// Counts a request of a key, and returns `false` over the limit of the window.
    pub(crate) fn allow(&self, key: K) -> bool {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        if counts.len() > MAX_KEYS {
            counts.retain(|_, (start, _)| now.duration_since(*start) < self.window);
        }
        let (start, count) = counts.entry(key).or_insert((now, 0));
        if now.duration_since(*start) >= self.window {
            (*start, *count) = (now, 0);
        }
        *count += 1;
        *count <= self.rate
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Rates;

    /// A key over its rate is refused, and the count of another key does not change.
    #[test]
    fn a_key_over_its_rate_is_refused_and_another_key_is_not() {
        // given
        let rates = Rates::new(2, Duration::from_secs(60));

        // when
        let first = [rates.allow("a"), rates.allow("a"), rates.allow("a")];
        let other = rates.allow("b");

        // then
        assert_eq!(first, [true, true, false]);
        assert!(other);
    }

    /// A new window starts the count of a key again.
    #[test]
    fn a_new_window_counts_again() {
        // given
        let rates = Rates::new(1, Duration::ZERO);

        // when
        let counts = [rates.allow(1), rates.allow(1)];

        // then
        assert_eq!(counts, [true, true]);
    }
}
