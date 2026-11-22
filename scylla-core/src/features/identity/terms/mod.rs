//! The terms feature: the acceptances of the terms of service by the users. A sign-up with a
//! password accepts the current version, and a user accepts a new version with `AcceptTerms`. The
//! acceptances of a deleted account stay as the proof.

mod service;
mod store;

pub(crate) use service::{accept, check_sign_up, keep_as_proof, record_sign_up, state};
