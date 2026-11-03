//! The `scylla-agent` binary: `join` makes the key pair and joins an App, and `run` serves its
//! jobs until SIGTERM or Ctrl+C.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use scylla_agent::{AgentConfig, AgentError, Settings};

/// The command line of `scylla-agent`.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// The file of the agent: its App and its private key.
    #[arg(
        long,
        env = "SCYLLA_AGENT_CONFIG",
        default_value = "/var/lib/scylla-agent/agent.json",
        global = true
    )]
    config: PathBuf,
    /// The command.
    #[command(subcommand)]
    command: Command,
}

/// The commands of the agent.
#[derive(Subcommand)]
enum Command {
    /// Makes a key pair, joins the App of a join code, and writes the file of the agent.
    Join {
        /// The URL of the server.
        url: String,
        /// The join code of the App.
        code: String,
    },
    /// Connects to the server and runs the jobs until SIGTERM or Ctrl+C.
    Run {
        /// The folder of the work folders of the jobs.
        #[arg(
            long,
            env = "SCYLLA_AGENT_WORKSPACE_ROOT",
            default_value = "/var/lib/scylla-agent/work"
        )]
        workspace_root: PathBuf,
        /// The largest number of jobs at the same time.
        #[arg(long, env = "SCYLLA_AGENT_CAPACITY", default_value_t = 1)]
        capacity: u32,
        /// The container runtime, for example docker. Not set runs no container step.
        #[arg(long, env = "SCYLLA_AGENT_CONTAINER_RUNTIME")]
        container_runtime: Option<String>,
        /// Keeps the work folder of each job after its end.
        #[arg(long, env = "SCYLLA_AGENT_KEEP_WORKSPACE")]
        keep_workspace: bool,
    },
}

/// Reads the command line, then runs the command.
#[tokio::main]
async fn main() -> Result<(), AgentError> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    match cli.command {
        Command::Join { url, code } => {
            let config = scylla_agent::join(&url, &code).await?;
            config.save(&cli.config)?;
            println!(
                "joined the app {}; the file is {}",
                config.app_id,
                cli.config.display()
            );
            Ok(())
        }
        Command::Run {
            workspace_root,
            capacity,
            container_runtime,
            keep_workspace,
        } => {
            let config = AgentConfig::load(&cli.config)?;
            let settings = Settings {
                workspace_root,
                capacity: capacity.max(1),
                container_runtime,
                keep_workspace,
            };
            scylla_agent::run(config, settings, stop()).await
        }
    }
}

/// Waits for SIGTERM or Ctrl+C.
async fn stop() {
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        () = terminate => {}
    }
}
