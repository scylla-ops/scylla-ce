//! Compiles the public protos and gives their folder to dependents as `DEP_SCYLLA_PROTO_INCLUDE`.

use std::env;
use std::path::PathBuf;

/// The proto files, relative to the `proto` folder.
const PROTOS: &[&str] = &[
    "scylla/access/v1/access_service.proto",
    "scylla/accounts/v1/account_service.proto",
    "scylla/accounts/v1/profile_service.proto",
    "scylla/accounts/v1/user.proto",
    "scylla/accounts/v1/user_admin_service.proto",
    "scylla/agents/v1/agent_service.proto",
    "scylla/auth/v1/assertion.proto",
    "scylla/auth/v1/token_service.proto",
    "scylla/capabilities/v1/capability_service.proto",
    "scylla/common/v1/id.proto",
    "scylla/common/v1/page.proto",
    "scylla/projects/v1/project.proto",
    "scylla/projects/v1/project_service.proto",
    "scylla/projects/v1/secret.proto",
    "scylla/projects/v1/secret_service.proto",
    "scylla/workspaces/v1/invitation_service.proto",
    "scylla/workspaces/v1/workspace.proto",
    "scylla/workspaces/v1/workspace_service.proto",
];

/// Compiles the protos, then prints the include folder for the dependent build scripts.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?)
        .join("..")
        .join("proto");
    let files: Vec<PathBuf> = PROTOS.iter().map(|file| root.join(file)).collect();
    tonic_prost_build::configure().compile_protos(&files, std::slice::from_ref(&root))?;
    println!("cargo::metadata=include={}", root.display());
    Ok(())
}
