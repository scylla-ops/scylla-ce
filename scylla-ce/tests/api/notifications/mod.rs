//! The tests of the notifications: the delivery of a fact, the inbox, the privacy of a row, the
//! kinds of the core, and the contract for an edition.

mod access_kinds;
mod conformance;
mod contract;
mod delivery;
mod inbox;
mod invitations;
mod privacy;
mod purge;
mod security_kinds;
mod support;

use support::{kinds_of, row_id, row_of};
