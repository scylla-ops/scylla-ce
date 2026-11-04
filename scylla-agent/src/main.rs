//! The `scylla-agent` binary: `join` makes the key pair and joins an App, and `run` serves its
//! jobs until a stop: Ctrl+C, SIGTERM on Unix, Ctrl+Break, the close of the console or a
//! shutdown on Windows. `run` joins first when the file of the agent does not exist and a join
//! code is set. With `--service`, `run` is the Windows service `ScyllaAgent`.

#[cfg(windows)]
mod service;

use std::future::Future;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use scylla_agent::{AgentError, Settings};

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
    Run(RunArgs),
}

/// The options of `run`.
#[derive(Args)]
struct RunArgs {
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
    /// The URL of the server, for a join before the run when the file of the agent does not
    /// exist.
    #[arg(long, env = "SCYLLA_URL", requires = "join_code")]
    url: Option<String>,
    /// The join code, for a join before the run when the file of the agent does not exist.
    #[arg(
        long,
        env = "SCYLLA_JOIN_CODE",
        hide_env_values = true,
        requires = "url"
    )]
    join_code: Option<String>,
    /// Runs as the Windows service `ScyllaAgent`, started by the service control manager.
    #[cfg(windows)]
    #[arg(long)]
    service: bool,
}

/// Reads the command line, then runs the command.
fn main() -> Result<(), AgentError> {
    let cli = Cli::parse();
    match cli.command {
        Command::Join { url, code } => {
            logs();
            runtime()?.block_on(join(&cli.config, &url, &code))
        }
        Command::Run(args) => {
            #[cfg(windows)]
            if args.service {
                return service::start(cli.config, args);
            }
            logs();
            runtime()?.block_on(run(&cli.config, args, stop()))
        }
    }
}

/// Writes the logs to the standard error.
fn logs() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
}

/// Makes the runtime of the agent.
fn runtime() -> Result<tokio::runtime::Runtime, AgentError> {
    tokio::runtime::Runtime::new().map_err(|error| AgentError::System(error.to_string()))
}

/// Joins the App of a join code, and writes the file of the agent.
async fn join(path: &Path, url: &str, code: &str) -> Result<(), AgentError> {
    let config = scylla_agent::join(url, code).await?;
    config.save(path)?;
    println!(
        "joined the app {}; the file is {}",
        config.app_id,
        path.display()
    );
    Ok(())
}

/// Joins first when the file of the agent does not exist and the options have a join code,
/// then runs the jobs until `stop`.
async fn run(path: &Path, args: RunArgs, stop: impl Future<Output = ()>) -> Result<(), AgentError> {
    let join_with = args.url.as_deref().zip(args.join_code.as_deref());
    let (config, joined) = scylla_agent::load_or_join(path, join_with).await?;
    if joined {
        println!(
            "joined the app {}; the file is {}",
            config.app_id,
            path.display()
        );
    }
    let settings = Settings {
        workspace_root: args.workspace_root,
        capacity: args.capacity.max(1),
        container_runtime: args.container_runtime,
        keep_workspace: args.keep_workspace,
    };
    scylla_agent::run(config, settings, stop).await
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
