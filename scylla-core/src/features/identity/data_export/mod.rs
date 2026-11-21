//! The export of the data of a user (`ExportAccountData`): its account rows and the parts of the
//! edition, as one JSON document.

mod service;
mod store;

pub(crate) use service::export;
