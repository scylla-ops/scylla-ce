fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure().compile_protos(
        &[
            "../proto/scylla/capabilities/v1/capability_service.proto",
            "../proto/scylla/projects/v1/project.proto",
            "../proto/scylla/projects/v1/project_service.proto",
        ],
        &["../proto"],
    )?;
    Ok(())
}
