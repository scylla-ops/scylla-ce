//! The rate limit of the records of the consent: the route needs no token, so each IP address
//! has a number of records in each window.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The most records from one IP address in one window.
const RATE: u32 = 30;

/// The window of the rate limit.
const WINDOW: Duration = Duration::from_secs(60);

/// The records of each IP address in the current window. A request with no known address counts
/// in one shared window.
#[derive(Default)]
pub(super) struct Rates {
    /// The start of the window and the count of each address.
    counts: Mutex<HashMap<Option<IpAddr>, (Instant, u32)>>,
}

impl Rates {
    /// Counts a record of an address, and returns `false` over the limit of the window.
    pub(super) fn allow(&self, ip: Option<IpAddr>) -> bool {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        if counts.len() > 10_000 {
            counts.retain(|_, (start, _)| now.duration_since(*start) < WINDOW);
        }
        let (start, count) = counts.entry(ip).or_insert((now, 0));
        if now.duration_since(*start) >= WINDOW {
            (*start, *count) = (now, 0);
        }
        *count += 1;
        *count <= RATE
    }
}
