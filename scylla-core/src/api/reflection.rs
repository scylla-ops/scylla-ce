//! The gRPC reflection v1 and v1alpha: the services and the messages of the core and of the
//! edition, for clients such as `grpcurl` and the MCP bridge.

use tonic::service::RoutesBuilder;
use tonic_reflection::server::{Builder, Error as ReflectionError};

use crate::Error;

/// Adds the reflection services v1 and v1alpha, with the descriptors of the core and of the
/// edition.
pub(super) fn add(routes: &mut RoutesBuilder, edition: &[&'static [u8]]) -> Result<(), Error> {
    let v1 = builder(edition).build_v1().map_err(failed)?;
    let v1alpha = builder(edition).build_v1alpha().map_err(failed)?;
    routes.add_service(v1).add_service(v1alpha);
    Ok(())
}

/// Makes a reflection builder with the descriptor set of the core, then each set of the edition.
fn builder(edition: &[&'static [u8]]) -> Builder<'static> {
    let core = Builder::configure()
        .register_encoded_file_descriptor_set(scylla_proto::FILE_DESCRIPTOR_SET);
    edition.iter().copied().fold(core, |builder, set| {
        builder.register_encoded_file_descriptor_set(set)
    })
}

/// Makes the error of a descriptor set that the reflection cannot read.
fn failed(error: ReflectionError) -> Error {
    Error::Internal(format!("gRPC reflection: {error}"))
}
