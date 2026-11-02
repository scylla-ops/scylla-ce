//! The projects feature: the `projects` table and `ProjectService`.

mod grpc;
mod plugin;
mod service;
mod store;

#[cfg(test)]
mod tests;

pub(crate) use plugin::Projects;
