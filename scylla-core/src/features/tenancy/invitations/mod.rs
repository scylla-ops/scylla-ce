//! The invitations feature: a link by mail that gives a role in a workspace, and
//! `InvitationService`.

mod grpc;
mod manage;
mod service;
mod store;

pub(crate) use grpc::server;
pub(crate) use service::{accept, find_open, open_for_email, Pick};
