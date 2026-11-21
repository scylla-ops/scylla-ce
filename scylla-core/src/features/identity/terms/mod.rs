//! The terms feature: the acceptances of the terms of service by the users. A sign-up with a
//! password accepts the current version, and a user accepts a new version with `AcceptTerms`.

mod service;
mod store;

pub(crate) use service::{accept, check_sign_up, record_sign_up, state};
