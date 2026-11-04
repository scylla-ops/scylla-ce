//! The `scylla-agent` binary: `join` makes the key pair and joins an App, and `run` serves its
//! jobs until a stop: Ctrl+C, SIGTERM on Unix, Ctrl+Break, the close of the console or a
//! shutdown on Windows.

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
        default_value_os_t = home().join("agent.json"),
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
    /// Connects to the server and runs the jobs until a stop.
    Run {
        /// The folder of the work folders of the jobs.
        #[arg(
            long,
            env = "SCYLLA_AGENT_WORKSPACE_ROOT",
            default_value_os_t = home().join("work")
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

/// The folder of the agent: `/var/lib/scylla-agent` on Unix, and
/// `%LOCALAPPDATA%\scylla-agent` on Windows.
fn home() -> PathBuf {
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("scylla-agent");
    }
    PathBuf::from("/var/lib/scylla-agent")
}

/// Waits for Ctrl+C or a stop of the system.
async fn stop() {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        () = system_stop() => {}
    }
}

/// Waits for SIGTERM.
#[cfg(unix)]
async fn system_stop() {
    if let Ok(mut signal) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        signal.recv().await;
    }
}

/// Waits for Ctrl+Break, the close of the console, or a shutdown.
#[cfg(windows)]
async fn system_stop() {
    use tokio::signal::windows::{ctrl_break, ctrl_close, ctrl_shutdown};

    let (Ok(mut brk), Ok(mut close), Ok(mut shutdown)) =
        (ctrl_break(), ctrl_close(), ctrl_shutdown())
    else {
        return;
    };
    tokio::select! {
        _ = brk.recv() => {}
        _ = close.recv() => {}
        _ = shutdown.recv() => {}
    }
}
