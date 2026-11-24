//! The agent as the Windows service `ScyllaAgent`: the service control manager starts it, and a
//! stop of the service or a shutdown stops the jobs. The logs go to a file for each day in the
//! folder of the file of the agent.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use scylla_agent::AgentError;
use tokio::sync::oneshot;
use tracing_appender::rolling::{Builder, Rotation};
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

use crate::RunArgs;

/// The name of the service.
const NAME: &str = "ScyllaAgent";

/// The number of daily log files that stay.
const LOG_FILES: usize = 7;

/// The file of the agent and the options of `run`, for the main function of the service.
static START: Mutex<Option<(PathBuf, RunArgs)>> = Mutex::new(None);

define_windows_service!(ffi_service_main, service_main);

/// Gives this process to the service control manager, which then calls the main function of
/// the service. Returns after the stop of the service.
pub(crate) fn start(config: PathBuf, args: RunArgs) -> Result<(), AgentError> {
    if let Ok(mut start) = START.lock() {
        *start = Some((config, args));
    }
    service_dispatcher::start(NAME, ffi_service_main).map_err(system)
}

/// The main function of the service.
fn service_main(_arguments: Vec<OsString>) {
    let Some((config, args)) = START.lock().ok().and_then(|mut start| start.take()) else {
        return;
    };
    if let Err(error) = serve(&config, args) {
        tracing::error!(%error, "the service stopped");
    }
}

/// Tells the service control manager that the service runs, runs the jobs until a stop of the
/// service or a shutdown, then tells it that the service stopped.
fn serve(config: &Path, args: RunArgs) -> Result<(), AgentError> {
    logs(config)?;
    let (stop, stopped) = oneshot::channel();
    let stop = Mutex::new(Some(stop));
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            if let Some(stop) = stop.lock().ok().and_then(|mut stop| stop.take()) {
                let _ = stop.send(());
            }
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let status = service_control_handler::register(NAME, handler).map_err(system)?;
    let accepted = ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN;
    status
        .set_service_status(state(ServiceState::Running, accepted, 0))
        .map_err(system)?;
    let stopped = async {
        let _ = stopped.await;
    };
    let result = crate::runtime()?.block_on(crate::run(config, args, stopped));
    let exit = u32::from(result.is_err());
    let end = state(ServiceState::Stopped, ServiceControlAccept::empty(), exit);
    status.set_service_status(end).map_err(system)?;
    result
}

/// Writes the logs to a file for each day, `agent.<date>.log`, next to the file of the agent.
fn logs(config: &Path) -> Result<(), AgentError> {
    let folder = config.parent().unwrap_or(Path::new("."));
    let file = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("agent")
        .filename_suffix("log")
        .max_log_files(LOG_FILES)
        .build(folder)
        .map_err(system)?;
    tracing_subscriber::fmt()
        .with_writer(file)
        .with_ansi(false)
        .init();
    Ok(())
}

/// Makes a state of the service for the service control manager.
fn state(current: ServiceState, accepted: ServiceControlAccept, exit: u32) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: current,
        controls_accepted: accepted,
        exit_code: ServiceExitCode::Win32(exit),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    }
}

/// Makes an error of the system.
fn system(error: impl std::fmt::Display) -> AgentError {
    AgentError::System(error.to_string())
}
