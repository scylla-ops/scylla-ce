#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("invalid control-plane URL `{url}`: {message}")]
    InvalidUrl { url: String, message: String },

    #[error("failed to connect to control plane: {0}")]
    Connection(#[from] tonic::transport::Error),

    #[error("agent stream closed unexpectedly")]
    StreamClosed,

    #[error("gRPC error: {0}")]
    Status(#[from] tonic::Status),

    #[error("invalid bearer token metadata: {0}")]
    InvalidToken(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error("node {node_id} failed with exit code {exit_code}")]
    NodeFailed { node_id: String, exit_code: i32 },

    #[error("node {node_id} was killed by signal")]
    NodeKilled { node_id: String },

    #[error("execution cancelled")]
    Cancelled,

    #[error("dangling dependencies: not all nodes could be scheduled (possible cycle)")]
    DanglingDeps,

    #[error("failed to spawn `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to wait for the step: {0}")]
    Wait(#[source] std::io::Error),

    #[error("workspace I/O error: {0}")]
    Workspace(#[source] std::io::Error),

    #[error("node {node_id} working directory escaped the job workspace")]
    WorkspaceEscape { node_id: String },

    #[error("failed to publish status: {0}")]
    Publish(String),

    #[error("node task panicked: {message}")]
    NodeTaskPanic { message: String },
}
