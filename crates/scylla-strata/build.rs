use std::env;
use std::path::PathBuf;

const SERVICES: &[&str] = &[
    "scylla/auth/v1/auth.proto",
    "scylla/user/v1/user.proto",
    "scylla/organization/v1/organization.proto",
    "scylla/project/v1/project.proto",
    "scylla/authz/v1/grant.proto",
    "scylla/authz/v1/role.proto",
    "scylla/pipeline/v1/pipeline.proto",
    "scylla/secret/v1/secret.proto",
    "scylla/job/v1/job.proto",
    "scylla/app/v1/app.proto",
    "scylla/app/v1/app_auth.proto",
    "scylla/agent/v1/agent_stream.proto",
    "scylla/agent/v1/agent_admin.proto",
    "scylla/trigger/v1/trigger.proto",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("../scylla-proto/proto");
    let protos: Vec<PathBuf> = SERVICES.iter().map(|p| root.join(p)).collect();
    for proto in &protos {
        println!("cargo:rerun-if-changed={}", proto.display());
    }
    tonic_prost_build::configure()
        .extern_path(".scylla", "::scylla_proto")
        .build_client(true)
        .build_server(true)
        .compile_protos(&protos, &[root])?;
    Ok(())
}
