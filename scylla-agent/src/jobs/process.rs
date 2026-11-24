//! The process of an attempt with each process that it starts, as one group: a process group on
//! Unix, a job object on Windows.

use std::io;
use std::process::ExitStatus;

use tokio::process::{ChildStderr, ChildStdout, Command};

#[cfg(windows)]
pub(super) use job::Process;
#[cfg(unix)]
pub(super) use unix::Process;

/// The group of an attempt on Unix: a stop signal to the group, then a kill.
#[cfg(unix)]
mod unix {
    use std::time::Duration;

    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;
    use tokio::process::Child;

    use super::{io, ChildStderr, ChildStdout, Command, ExitStatus};

    /// The time between the stop signal and the kill of a process group.
    const GRACE: Duration = Duration::from_secs(5);

    /// A started attempt, the leader of its process group.
    pub(in crate::jobs) struct Process {
        /// The first process.
        child: Child,
    }

    impl Process {
        /// Starts the command in a new process group.
        pub(in crate::jobs) fn spawn(mut command: Command) -> io::Result<Self> {
            command.process_group(0);
            command.spawn().map(|child| Self { child })
        }

        /// Takes the standard output.
        pub(in crate::jobs) fn stdout(&mut self) -> Option<ChildStdout> {
            self.child.stdout.take()
        }

        /// Takes the standard error.
        pub(in crate::jobs) fn stderr(&mut self) -> Option<ChildStderr> {
            self.child.stderr.take()
        }

        /// Waits for the end of the first process.
        pub(in crate::jobs) async fn wait(&mut self) -> io::Result<ExitStatus> {
            self.child.wait().await
        }

        /// Sends SIGTERM to the group, and SIGKILL after the grace time.
        pub(in crate::jobs) async fn stop(&mut self) {
            if let Some(pid) = self.child.id().and_then(|pid| i32::try_from(pid).ok()) {
                let _ = killpg(Pid::from_raw(pid), Signal::SIGTERM);
                if tokio::time::timeout(GRACE, self.child.wait())
                    .await
                    .is_err()
                {
                    let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
                }
            }
            let _ = self.child.wait().await;
        }
    }
}

/// The group of an attempt on Windows: a job object, which has no stop signal.
#[cfg(windows)]
mod job {
    use process_wrap::tokio::{ChildWrapper, CommandWrap, CreationFlags, JobObject};
    use windows::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP;

    use super::{io, ChildStderr, ChildStdout, Command, ExitStatus};

    /// A started attempt in its job object.
    pub(in crate::jobs) struct Process {
        /// The first process, in the job object.
        child: Box<dyn ChildWrapper>,
    }

    impl Process {
        /// Starts the command in a new job object, out of the Ctrl+C of the console of the agent.
        pub(in crate::jobs) fn spawn(command: Command) -> io::Result<Self> {
            CommandWrap::from(command)
                .wrap(CreationFlags(CREATE_NEW_PROCESS_GROUP))
                .wrap(JobObject)
                .spawn()
                .map(|child| Self { child })
        }

        /// Takes the standard output.
        pub(in crate::jobs) fn stdout(&mut self) -> Option<ChildStdout> {
            self.child.stdout().take()
        }

        /// Takes the standard error.
        pub(in crate::jobs) fn stderr(&mut self) -> Option<ChildStderr> {
            self.child.stderr().take()
        }

        /// Waits for the end of the first process, then of each process of the job.
        pub(in crate::jobs) async fn wait(&mut self) -> io::Result<ExitStatus> {
            self.child.wait().await
        }

        /// Ends each process of the job object at once.
        pub(in crate::jobs) async fn stop(&mut self) {
            let _ = self.child.start_kill();
            let _ = self.child.wait().await;
        }
    }
}
