//! The capabilities feature: `CapabilityService` tells a client which edition runs.

mod grpc;
mod plugin;

pub(crate) use plugin::Capabilities;
