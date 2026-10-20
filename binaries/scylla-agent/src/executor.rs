use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::fs;
use tokio::process::{Child, Command};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use scylla_domain::JobEvent;
use scylla_domain::domain::job::LogStream;
use scylla_domain::domain::pipeline::{DagPlan, ExecArg, PipelineNode, Shell, Step};

use crate::error::ExecutionError;
use crate::output::{self, NodeLog, Secrets};
use crate::reporter::StatusPublisher;

const TERM_GRACE: Duration = Duration::from_secs(5);

pub struct Executor {
    publisher: StatusPublisher,
    workspace_root: PathBuf,
    keep_workspace: bool,
    secrets: Arc<Secrets>,
}

impl Executor {
    pub fn new(
        publisher: StatusPublisher,
        workspace_root: PathBuf,
        keep_workspace: bool,
        secrets: Secrets,
    ) -> Self {
        Self {
            publisher,
            workspace_root,
            keep_workspace,
            secrets: Arc::new(secrets),
        }
    }

    /// Emits exactly one `JobStarted` and one terminal event. `cancel` stops the running steps;
    /// the nodes it stops or never starts get no event, and the end of the job cancels them.
    pub async fn run(
        &self,
        nodes: Vec<PipelineNode>,
        cancel: CancellationToken,
    ) -> Result<(), ExecutionError> {
        // Before any workspace I/O: a failure with no terminal event strands the job in `pending`.
        self.publisher.emit(JobEvent::JobStarted).await?;

        let (workspace, outcome) = match self.prepare_workspace().await {
            Ok(ws) => {
                let outcome = self.execute(&nodes, &cancel, &ws).await;
                (Some(ws), outcome)
            }
            Err(e) => (None, Err(e)),
        };

        let end = match &outcome {
            Ok(()) => JobEvent::JobCompleted,
            Err(err) => JobEvent::JobFailed {
                error: format!("job failed: {err}"),
            },
        };
        let finalize = self.publisher.emit(end).await;

        if !self.keep_workspace
            && let Some(ws) = &workspace
            && let Err(e) = fs::remove_dir_all(ws).await
        {
            warn!(error = %e, workspace = %ws.display(), "failed to remove job workspace");
        }
        finalize?;
        outcome
    }

    async fn prepare_workspace(&self) -> Result<PathBuf, ExecutionError> {
        let workspace = self.workspace_root.join(self.publisher.job_id());
        let with_path = |e: io::Error| {
            ExecutionError::Workspace(io::Error::new(
                e.kind(),
                format!("{}: {e}", workspace.display()),
            ))
        };
        fs::create_dir_all(&workspace).await.map_err(with_path)?;
        // Canonicalize once so the per-node prefix checks defeat symlink escapes.
        fs::canonicalize(&workspace).await.map_err(with_path)
    }

    async fn execute(
        &self,
        nodes: &[PipelineNode],
        cancel: &CancellationToken,
        workspace: &Path,
    ) -> Result<(), ExecutionError> {
        let publisher = &self.publisher;
        let stop = cancel.child_token();
        let mut plan = DagPlan::build(nodes);
        let mut running = JoinSet::new();
        let mut failure = None;

        loop {
            if !stop.is_cancelled() {
                for id in plan.drain_ready() {
                    publisher
                        .emit(JobEvent::NodeStarted {
                            node_id: id.to_owned(),
                        })
                        .await?;
                    let spec = plan.lookup(id).clone();
                    let log = NodeLog::new(publisher.clone(), id.to_owned(), self.secrets.clone());
                    let (workspace, stop) = (workspace.to_path_buf(), stop.clone());
                    running.spawn(async move {
                        let result = run_node(&spec, log, &workspace, stop).await;
                        (spec.id().to_string(), result)
                    });
                }
            }
            let Some(joined) = running.join_next().await else {
                break;
            };
            let (id, result) = joined.map_err(|e| ExecutionError::NodeTaskPanic {
                message: e.to_string(),
            })?;
            match &result {
                Ok(()) => {
                    info!(node_id = %id, "node completed");
                    plan.mark_completed(&id);
                }
                Err(ExecutionError::Cancelled) => {
                    info!(node_id = %id, "node stopped");
                    plan.mark_terminal(&id);
                    if cancel.is_cancelled() {
                        continue;
                    }
                }
                Err(err) => {
                    error!(node_id = %id, error = %err, "node did not complete");
                    plan.mark_terminal(&id);
                }
            }
            publisher.emit(node_event(id, &result)).await?;
            if let Err(err) = result
                && !matches!(err, ExecutionError::Cancelled)
            {
                stop.cancel();
                failure.get_or_insert(err);
            }
        }

        let stranded = !plan.is_exhausted();
        if failure.is_none() && cancel.is_cancelled() {
            return Err(ExecutionError::Cancelled);
        }
        skip_all_pending(&mut plan, publisher).await?;
        match failure {
            Some(err) => Err(err),
            None if stranded => Err(ExecutionError::DanglingDeps),
            None => Ok(()),
        }
    }
}

fn node_event(node_id: String, result: &Result<(), ExecutionError>) -> JobEvent {
    match result {
        Ok(()) => JobEvent::NodeCompleted { node_id },
        Err(ExecutionError::Cancelled) => JobEvent::NodeSkipped { node_id },
        Err(err) => JobEvent::NodeFailed {
            node_id,
            error: err.to_string(),
        },
    }
}

async fn skip_all_pending(
    plan: &mut DagPlan<'_>,
    publisher: &StatusPublisher,
) -> Result<(), ExecutionError> {
    let remaining: Vec<String> = plan.pending().map(str::to_string).collect();
    for id in remaining {
        publisher
            .emit(JobEvent::NodeSkipped {
                node_id: id.clone(),
            })
            .await?;
        plan.mark_terminal(&id);
    }
    Ok(())
}

/// The output reader finishes before the node's terminal event, so no log line follows it.
async fn run_node(
    spec: &PipelineNode,
    mut log: NodeLog,
    workspace: &Path,
    cancel: CancellationToken,
) -> Result<(), ExecutionError> {
    let (mut child, group) = match spawn(spec, log.job_id(), workspace).await {
        Ok(spawned) => spawned,
        Err(e) => {
            log.send(LogStream::Stderr, &e.to_string()).await;
            return Err(e);
        }
    };
    // INVARIANT: spawn pipes stdout and stderr.
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let output = tokio::spawn(output::forward(log, stdout, stderr));

    let result = tokio::select! {
        biased;
        status = child.wait() => exit_status_to_result(spec.id().as_str(), status),
        () = cancel.cancelled() => {
            group.signal(Signal::Term);
            if tokio::time::timeout(TERM_GRACE, child.wait()).await.is_err() {
                group.signal(Signal::Kill);
                let _ = child.wait().await;
            }
            Err(ExecutionError::Cancelled)
        }
    };
    group.kill().await;
    let _ = output.await;
    result
}

async fn spawn(
    spec: &PipelineNode,
    job_id: &str,
    workspace: &Path,
) -> Result<(Child, ProcessGroup), ExecutionError> {
    let node_id = spec.id().as_str();
    let cwd = working_dir(spec, workspace).await?;
    let (program, args) = program(spec, workspace).await?;
    let mut command = Command::new(on_agent_path(program));
    command
        .args(args)
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    configure_env(&mut command, spec, job_id, node_id, workspace);
    ProcessGroup::spawn(&mut command).map_err(|source| ExecutionError::Spawn {
        program: program.to_owned(),
        source,
    })
}

async fn working_dir(spec: &PipelineNode, workspace: &Path) -> Result<PathBuf, ExecutionError> {
    let escape = || ExecutionError::WorkspaceEscape {
        node_id: spec.id().to_string(),
    };
    let requested = match spec.working_dir() {
        Some(wd) => {
            // Before anything is created: `create_dir_all` would materialize an escaping path before the canonical check.
            if Path::new(wd.as_str())
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::RootDir))
            {
                return Err(escape());
            }
            workspace.join(wd.as_str())
        }
        None => workspace.to_path_buf(),
    };
    fs::create_dir_all(&requested)
        .await
        .map_err(ExecutionError::Workspace)?;
    // Canonical check: a symlink inside the workspace may resolve outside it.
    let cwd = fs::canonicalize(&requested)
        .await
        .map_err(ExecutionError::Workspace)?;
    if cwd.starts_with(workspace) {
        Ok(cwd)
    } else {
        Err(escape())
    }
}

/// Scripts run from a file, not `-c`: correct line numbers and no ARG_MAX limit.
async fn program<'a>(
    spec: &'a PipelineNode,
    workspace: &Path,
) -> Result<(&'a str, Vec<OsString>), ExecutionError> {
    match spec.step() {
        Step::Exec { command, args } => Ok((
            command.as_str(),
            args.iter()
                .map(ExecArg::as_str)
                .map(OsString::from)
                .collect(),
        )),
        Step::Script { script, shell } => {
            let dir = workspace.join(".scylla");
            fs::create_dir_all(&dir)
                .await
                .map_err(ExecutionError::Workspace)?;
            let path = dir.join(format!("{}.sh", spec.id().as_str()));
            fs::write(&path, script.as_str())
                .await
                .map_err(ExecutionError::Workspace)?;
            let (program, flags): (&str, &[&str]) = match shell {
                Shell::Sh => ("sh", &["-e"]),
                Shell::Bash => ("bash", &["--noprofile", "--norc", "-o", "pipefail", "-e"]),
            };
            let args = flags
                .iter()
                .map(OsString::from)
                .chain([path.into_os_string()])
                .collect();
            Ok((program, args))
        }
    }
}

/// The node env may set PATH for the step's own children; it must not change which program the agent starts.
fn on_agent_path(program: &str) -> PathBuf {
    if program.contains('/') {
        return program.into();
    }
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(program))
                .find(|path| is_executable(path))
        })
        .unwrap_or_else(|| program.into())
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// The inherited environment is cleared so the agent's own token and secrets never reach a job.
fn configure_env(
    command: &mut Command,
    spec: &PipelineNode,
    job_id: &str,
    node_id: &str,
    workspace: &Path,
) {
    command.env_clear();
    for key in ["PATH", "HOME", "LANG", "LC_ALL"] {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }
    command.env("TERM", "dumb");
    for ev in spec.env() {
        if let Some(value) = ev.literal_value() {
            command.env(ev.key(), value);
        }
    }
    // Injected last so it is authoritative.
    command.env("CI", "true");
    command.env("SCYLLA_WORKSPACE", workspace);
    command.env("SCYLLA_JOB_ID", job_id);
    command.env("SCYLLA_NODE_ID", node_id);
}

#[derive(Clone, Copy)]
enum Signal {
    Term,
    Kill,
}

/// Each step leads its own process group; dropping the guard kills the whole group, background children included.
struct ProcessGroup(Option<i32>);

const KILL_ROUNDS: u32 = 50;
const KILL_PAUSE: Duration = Duration::from_millis(20);

impl ProcessGroup {
    fn spawn(command: &mut Command) -> io::Result<(Child, Self)> {
        #[cfg(unix)]
        command.process_group(0);
        let child = command.kill_on_drop(true).spawn()?;
        let pgid = child.id().and_then(|id| i32::try_from(id).ok());
        Ok((child, Self(pgid)))
    }

    /// A member that forks while the group is signalled can leave a child the signal missed,
    /// so the group is signalled again until no member is left. The shell is reaped first.
    async fn kill(mut self) {
        for _ in 0..KILL_ROUNDS {
            if !self.signal(Signal::Kill) {
                break;
            }
            tokio::time::sleep(KILL_PAUSE).await;
        }
        self.0 = None;
    }

    /// True when a member of the group got the signal.
    #[cfg(unix)]
    #[allow(unsafe_code)]
    fn signal(&self, signal: Signal) -> bool {
        let Some(pgid) = self.0 else {
            return false;
        };
        let sig = match signal {
            Signal::Term => libc::SIGTERM,
            Signal::Kill => libc::SIGKILL,
        };
        // SAFETY: kill(2) takes no pointers; a negative pid targets the process group.
        unsafe { libc::kill(-pgid, sig) == 0 }
    }

    #[cfg(not(unix))]
    fn signal(&self, _signal: Signal) -> bool {
        false
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let _ = self.signal(Signal::Kill);
    }
}

fn exit_status_to_result(
    node_id: &str,
    status: io::Result<std::process::ExitStatus>,
) -> Result<(), ExecutionError> {
    let status = status.map_err(ExecutionError::Wait)?;
    if status.success() {
        Ok(())
    } else {
        Err(match status.code() {
            Some(code) => ExecutionError::NodeFailed {
                node_id: node_id.to_string(),
                exit_code: code,
            },
            None => ExecutionError::NodeKilled {
                node_id: node_id.to_string(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scylla_domain::domain::pipeline::{EnvKey, EnvVar, NodeId, WorkingDir};
    use scylla_proto::agent::v1::job_status::Event;
    use scylla_proto::agent::v1::{JobLogLine, JobStatus, agent_up};
    use std::time::Instant;
    use tokio::sync::mpsc;

    struct Run {
        result: Result<(), ExecutionError>,
        statuses: Vec<JobStatus>,
        logs: Vec<JobLogLine>,
    }

    impl Run {
        fn events(&self) -> Vec<String> {
            self.statuses
                .iter()
                .filter_map(|s| s.event.as_ref().map(label))
                .collect()
        }

        fn has(&self, event: &str) -> bool {
            self.events().iter().any(|e| e == event)
        }

        fn text(&self) -> String {
            self.logs
                .iter()
                .flat_map(|l| [l.line.as_str(), "\n"])
                .collect()
        }
    }

    fn label(event: &Event) -> String {
        let node = |id: &Option<scylla_proto::common::v1::NodeId>| {
            id.as_ref().map(|n| n.value.clone()).unwrap_or_default()
        };
        match event {
            Event::JobStarted(_) => "job_started".into(),
            Event::JobCompleted(_) => "job_completed".into(),
            Event::JobFailed(_) => "job_failed".into(),
            Event::NodeStarted(n) => format!("node_started:{}", node(&n.node_id)),
            Event::NodeCompleted(n) => format!("node_completed:{}", node(&n.node_id)),
            Event::NodeFailed(n) => format!("node_failed:{}", node(&n.node_id)),
            Event::NodeSkipped(n) => format!("node_skipped:{}", node(&n.node_id)),
        }
    }

    fn tmp_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("scylla-exec-{tag}-{}", std::process::id()))
    }

    fn node(
        id: &str,
        deps: &[&str],
        step: Step,
        working_dir: Option<&str>,
        env: &[(&str, &str)],
    ) -> PipelineNode {
        let node_id = NodeId::new(id).unwrap();
        let deps = deps.iter().map(|d| NodeId::new(*d).unwrap()).collect();
        let working_dir = working_dir.map(|w| WorkingDir::new(w).unwrap());
        let env = env
            .iter()
            .map(|(k, v)| EnvVar::literal(EnvKey::new(*k).unwrap(), *v).unwrap())
            .collect();
        PipelineNode::new(node_id, deps, step, working_dir, env)
    }

    fn script_node(id: &str, deps: &[&str], body: &str) -> PipelineNode {
        node(id, deps, script(body), None, &[])
    }

    fn script(body: &str) -> Step {
        Step::script(body.to_string(), Shell::Sh).unwrap()
    }

    async fn execute(
        root: &Path,
        job_id: &str,
        keep_workspace: bool,
        secrets: Secrets,
        nodes: Vec<PipelineNode>,
        cancel: CancellationToken,
    ) -> Run {
        let (tx, mut rx) = mpsc::channel(64);
        let collector = tokio::spawn(async move {
            let mut frames = Vec::new();
            while let Some(frame) = rx.recv().await {
                frames.push(frame);
            }
            frames
        });
        let result = Executor::new(
            StatusPublisher::new(tx, job_id.into(), CancellationToken::new()),
            root.to_path_buf(),
            keep_workspace,
            secrets,
        )
        .run(nodes, cancel)
        .await;
        let mut run = Run {
            result,
            statuses: Vec::new(),
            logs: Vec::new(),
        };
        for frame in collector.await.unwrap() {
            match frame.payload {
                Some(agent_up::Payload::Status(s)) => run.statuses.push(s),
                Some(agent_up::Payload::Log(l)) => run.logs.push(l),
                Some(agent_up::Payload::Hello(_)) | None => {}
            }
        }
        run
    }

    async fn simple(tag: &str, nodes: Vec<PipelineNode>) -> Run {
        let root = tmp_root(tag);
        let run = execute(
            &root,
            tag,
            false,
            Secrets::default(),
            nodes,
            CancellationToken::new(),
        )
        .await;
        let _ = std::fs::remove_dir_all(&root);
        run
    }

    fn alive(pid: &str) -> bool {
        let out = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .unwrap();
        let stat = String::from_utf8_lossy(&out.stdout);
        let stat = stat.trim();
        !stat.is_empty() && !stat.starts_with('Z')
    }

    async fn dies(pid: &str) -> bool {
        for _ in 0..40 {
            if !alive(pid) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }

    #[test]
    fn node_event_reports_the_real_result() {
        let event = |r| node_event("n".into(), &r);
        assert!(matches!(event(Ok(())), JobEvent::NodeCompleted { .. }));
        assert!(matches!(
            event(Err(ExecutionError::Cancelled)),
            JobEvent::NodeSkipped { .. }
        ));
        assert!(matches!(
            event(Err(ExecutionError::NodeFailed {
                node_id: "n".into(),
                exit_code: 1
            })),
            JobEvent::NodeFailed { .. }
        ));
    }

    #[tokio::test]
    async fn masked_values_are_redacted_in_job_logs() {
        let root = tmp_root("redact");
        let run = execute(
            &root,
            "job-redact",
            false,
            Secrets::new(["s3cr3t-value\n".to_string()]),
            vec![node(
                "n1",
                &[],
                script("echo \"leaking $TOKEN here\""),
                None,
                &[("TOKEN", "s3cr3t-value")],
            )],
            CancellationToken::new(),
        )
        .await;
        let _ = std::fs::remove_dir_all(&root);
        let logs = run.text();

        run.result.unwrap();
        assert!(logs.contains("leaking *** here"), "logs: {logs:?}");
        assert!(!logs.contains("s3cr3t-value"), "logs: {logs:?}");
    }

    #[tokio::test]
    async fn agent_environment_does_not_leak_into_jobs() {
        const KEPT: [&str; 8] = [
            "PATH", "HOME", "LANG", "LC_ALL", "TERM", "CI", "PWD", "SHLVL",
        ];
        let probe = std::env::vars()
            .map(|(k, _)| k)
            .find(|k| {
                !KEPT.contains(&k.as_str())
                    && !k.starts_with("SCYLLA_")
                    && k != "_"
                    && k != "OLDPWD"
                    && !k.is_empty()
                    && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
            .unwrap_or_else(|| "SCYLLA_NO_SUCH_VAR".to_string());

        let body = format!("echo \"LEAK=[${{{probe}}}]\"; echo \"JOB=[$SCYLLA_JOB_ID]\"");
        let run = simple("job-env", vec![script_node("n1", &[], &body)]).await;
        let logs = run.text();

        assert!(
            logs.contains("LEAK=[]"),
            "the agent's own env var `{probe}` must not leak into the job; logs: {logs:?}",
        );
        assert!(logs.contains("JOB=[job-env]"), "logs: {logs:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn working_directory_escaping_the_workspace_is_rejected() {
        let root = tmp_root("escape");
        let outside = tmp_root("escape-outside");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&outside).unwrap();
        let workspace = root.join("job-escape");
        std::fs::create_dir_all(&workspace).unwrap();
        std::os::unix::fs::symlink(&outside, workspace.join("out")).unwrap();

        let run = execute(
            &root,
            "job-escape",
            false,
            Secrets::default(),
            vec![node("n1", &[], script("echo hi"), Some("out"), &[])],
            CancellationToken::new(),
        )
        .await;
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);

        assert!(matches!(
            run.result,
            Err(ExecutionError::WorkspaceEscape { .. })
        ));
        assert!(run.has("node_failed:n1"));
        assert!(run.has("job_failed"));
        let log = &run.logs[0];
        assert_eq!(
            log.stream,
            scylla_proto::convert::log_stream_to_proto(LogStream::Stderr) as i32
        );
        assert!(log.line.contains("escaped the job workspace"), "{log:?}");
    }

    #[tokio::test]
    async fn a_nonzero_exit_code_maps_to_node_failed() {
        let run = simple("exit", vec![script_node("n1", &[], "exit 3")]).await;

        assert!(
            matches!(
                run.result,
                Err(ExecutionError::NodeFailed { exit_code: 3, .. })
            ),
            "exit code must be preserved, got {:?}",
            run.result,
        );
        assert!(run.has("job_failed"));
    }

    #[tokio::test]
    async fn a_dependent_node_is_skipped_when_its_dependency_fails() {
        let run = simple(
            "skip",
            vec![
                script_node("build", &[], "exit 1"),
                script_node("test", &["build"], "echo should-not-run"),
            ],
        )
        .await;

        assert!(run.has("node_skipped:test"));
        assert!(!run.text().contains("should-not-run"));
        assert!(run.has("job_failed"));
    }

    #[tokio::test]
    async fn a_sibling_that_completed_is_reported_completed() {
        let run = simple(
            "sibling",
            vec![
                script_node(
                    "a",
                    &[],
                    "while [ ! -f b.done ]; do sleep 0.05; done; sleep 0.5; exit 1",
                ),
                script_node("b", &[], "echo b-done; touch b.done"),
            ],
        )
        .await;

        assert!(run.text().contains("b-done"));
        assert!(run.has("node_completed:b"), "{:?}", run.events());
        assert!(run.has("node_failed:a"), "{:?}", run.events());
        assert_eq!(run.events().last().unwrap(), "job_failed");
    }

    #[tokio::test]
    async fn invalid_utf8_output_does_not_stop_the_node() {
        let run = simple(
            "utf8",
            vec![script_node(
                "n1",
                &[],
                "printf '\\377\\376bad\\n'; echo after; seq 1 20000",
            )],
        )
        .await;
        let logs = run.text();

        run.result.unwrap();
        assert!(logs.contains("\u{FFFD}\u{FFFD}bad"));
        assert!(logs.contains("after\n"));
        assert!(logs.contains("\n20000\n"));
    }

    #[tokio::test]
    async fn one_node_stamps_its_lines_in_read_order() {
        let run = simple(
            "order",
            vec![script_node(
                "n1",
                &[],
                "seq 1 2000; echo out1; sleep 0.05; echo err1 >&2; sleep 0.05; echo out2",
            )],
        )
        .await;
        run.result.unwrap();

        let stamps: Vec<_> = run
            .logs
            .iter()
            .map(|l| {
                let t = l.timestamp.unwrap();
                (t.seconds, t.nanos)
            })
            .collect();
        assert!(stamps.windows(2).all(|w| w[0] < w[1]));

        let mut logs = run.logs;
        logs.sort_by_key(|l| {
            let t = l.timestamp.unwrap();
            (t.seconds, t.nanos)
        });
        let tail: Vec<_> = logs
            .iter()
            .rev()
            .take(3)
            .rev()
            .map(|l| l.line.as_str())
            .collect();
        assert_eq!(tail, ["out1", "err1", "out2"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_background_process_dies_with_its_node() {
        let root = tmp_root("background");
        let started = Instant::now();
        let run = execute(
            &root,
            "job-bg",
            true,
            Secrets::default(),
            vec![script_node(
                "n1",
                &[],
                "sleep 30 & echo $! > bg.pid; echo done",
            )],
            CancellationToken::new(),
        )
        .await;
        let elapsed = started.elapsed();
        let pid = std::fs::read_to_string(root.join("job-bg").join("bg.pid")).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        run.result.unwrap();
        assert!(elapsed < Duration::from_secs(10), "took {elapsed:?}");
        assert!(dies(pid.trim()).await, "background pid {pid} survived");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_external_cancel_stops_the_job() {
        let root = tmp_root("cancel");
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        let pid_file = root.join("job-cancel").join("bg.pid");
        let cancelled = tokio::spawn(async move {
            while !pid_file.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            trigger.cancel();
            Instant::now()
        });
        let run = execute(
            &root,
            "job-cancel",
            true,
            Secrets::default(),
            vec![
                script_node("n1", &[], "sleep 30 & echo $! > bg.pid; sleep 30"),
                script_node("n2", &["n1"], "echo never"),
            ],
            cancel,
        )
        .await;
        let elapsed = cancelled.await.unwrap().elapsed();
        let pid = std::fs::read_to_string(root.join("job-cancel").join("bg.pid")).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert!(matches!(run.result, Err(ExecutionError::Cancelled)));
        assert!(
            elapsed < TERM_GRACE + Duration::from_secs(2),
            "took {elapsed:?}"
        );
        assert_eq!(
            run.events(),
            ["job_started", "node_started:n1", "job_failed"]
        );
        match run.statuses.last().and_then(|s| s.event.as_ref()) {
            Some(Event::JobFailed(f)) => assert_eq!(f.error, "job failed: execution cancelled"),
            other => panic!("expected JobFailed, got {other:?}"),
        }
        assert!(dies(pid.trim()).await, "background pid {pid} survived");
    }

    #[tokio::test]
    async fn a_job_cancelled_before_it_starts_reports_only_started_and_failed() {
        let root = tmp_root("precancel");
        let cancel = CancellationToken::new();
        cancel.cancel();
        let run = execute(
            &root,
            "job-precancel",
            false,
            Secrets::default(),
            vec![script_node("n1", &[], "echo never")],
            cancel,
        )
        .await;
        let _ = std::fs::remove_dir_all(&root);

        assert!(matches!(run.result, Err(ExecutionError::Cancelled)));
        assert_eq!(run.events(), ["job_started", "job_failed"]);
        assert!(run.logs.is_empty());
    }

    #[tokio::test]
    async fn a_path_in_the_node_env_does_not_change_the_program_lookup() {
        let env = [("PATH", "/nope")];
        let run = simple(
            "path",
            vec![
                node("script", &[], script("echo hi-script"), None, &env),
                node(
                    "exec",
                    &[],
                    Step::exec("echo".into(), vec!["hi-exec".into()]).unwrap(),
                    None,
                    &env,
                ),
            ],
        )
        .await;
        let logs = run.text();

        run.result.unwrap();
        assert!(
            logs.contains("hi-script") && logs.contains("hi-exec"),
            "{logs:?}"
        );
    }

    #[tokio::test]
    async fn a_missing_program_fails_the_node_with_its_name() {
        let run = simple(
            "missing",
            vec![node(
                "n1",
                &[],
                Step::exec("scylla-no-such-program".into(), vec![]).unwrap(),
                None,
                &[],
            )],
        )
        .await;

        assert!(matches!(run.result, Err(ExecutionError::Spawn { .. })));
        assert!(
            run.text()
                .contains("failed to spawn `scylla-no-such-program`"),
            "{:?}",
            run.text()
        );
    }

    #[tokio::test]
    async fn workspace_failure_still_reports_started_and_failed() {
        let blocker = std::env::temp_dir().join(format!("scylla-exec-test-{}", std::process::id()));
        tokio::fs::write(&blocker, b"x").await.unwrap();
        let run = execute(
            &blocker.join("ws"),
            "job-1",
            false,
            Secrets::default(),
            vec![],
            CancellationToken::new(),
        )
        .await;
        let _ = tokio::fs::remove_file(&blocker).await;

        assert!(run.result.is_err(), "workspace creation must fail");
        assert_eq!(run.events(), ["job_started", "job_failed"]);
        let last_error = match run.statuses[1].event.as_ref() {
            Some(Event::JobFailed(f)) => f.error.as_str(),
            other => panic!("expected a terminal JobFailed, got {other:?}"),
        };
        assert!(
            last_error.contains("scylla-exec-test"),
            "the failure message must carry the offending path, got: {last_error}",
        );
    }
}
