//! The checkout of the code of a job: the agent fetches one commit into the work folder before
//! the first node, with a header that it never writes to a file, a URL or a log.

mod git;
#[cfg(test)]
mod origin;
#[cfg(test)]
mod tests;

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use scylla_proto::agents::v1::{DispatchCheckout, EventKind};
use scylla_proto::pipelines::v1::LogStream;
use tokio::process::Command;
use tokio::sync::watch;
use tokio::time::{sleep_until, Instant};

pub(crate) use git::{features, Git};

use super::events::{self, Event, Outbox};
use super::output;
use super::process::Process;
use git::Config;

/// The node of the log lines of the checkout.
const NODE: &str = ".checkout";

/// The longest time of a checkout.
const LIMIT: Duration = Duration::from_secs(3600);

/// The end of a checkout that did not complete.
pub(super) enum Failure {
    /// A step failed, with the reason.
    Failed(String),
    /// The server stopped the job.
    Halted,
}

/// One checkout: where it runs, what it masks, and where its messages go.
struct Runner<'a> {
    /// The work folder of the job.
    dir: &'a Path,
    /// The job.
    job: &'a str,
    /// The messages to the server.
    outbox: &'a Outbox,
    /// The settings of each command: no credential helper, and the header for the clone URL only.
    config: Config,
    /// The values that the log never shows.
    masks: Vec<String>,
    /// The end of the time of the checkout.
    deadline: Instant,
    /// The stop of the job by the server.
    stopped: watch::Receiver<bool>,
}

/// Fetches the commit of a job into its work folder with the git that the hello found, and sends
/// the events of the checkout and its lines on the node `.checkout`.
pub(super) async fn run(
    checkout: &DispatchCheckout,
    git: Option<Git>,
    dir: &Path,
    job: &str,
    outbox: &Outbox,
    stopped: watch::Receiver<bool>,
) -> Result<(), Failure> {
    // Tell the start, and get the commit.
    events::send(outbox, job, EventKind::CheckoutStarted, Event::default()).await;
    let config = Config::of(checkout, git);
    if config.header_in_args() {
        tracing::warn!(%job, "git is older than 2.31: the header of the checkout is in its arguments");
    }
    let mut runner = Runner {
        dir,
        job,
        outbox,
        config,
        masks: masks(&checkout.auth_header),
        deadline: Instant::now() + LIMIT,
        stopped,
    };
    let ended = fetch(&mut runner, checkout)
        .await
        .map_err(|failure| match failure {
            Failure::Failed(error) => Failure::Failed(output::mask(error, &runner.masks)),
            Failure::Halted => Failure::Halted,
        });

    // Tell the end, with the reason of a failure in the log and in the event.
    match &ended {
        Ok(()) => events::send(outbox, job, EventKind::CheckoutCompleted, Event::default()).await,
        Err(Failure::Failed(error)) => {
            let line = format!("the checkout failed: {error}");
            output::report(outbox, job, NODE, &line).await;
            let event = Event {
                error: error.clone(),
                ..Event::default()
            };
            events::send(outbox, job, EventKind::CheckoutFailed, event).await;
        }
        Err(Failure::Halted) => {}
    }
    ended
}

/// Makes a new repository, fetches the commit or its ref, detaches HEAD on it, then gets the
/// submodules and the LFS files when the dispatch asks.
async fn fetch(runner: &mut Runner<'_>, checkout: &DispatchCheckout) -> Result<(), Failure> {
    // Refuse a commit or a ref that git can read as an option.
    let sha = checkout.sha.to_ascii_lowercase();
    if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Failure::Failed(format!("the commit {sha} is not a SHA")));
    }
    let target = match checkout.fetch_ref.as_str() {
        "" => sha.as_str(),
        name if name.starts_with("refs/") && !name.contains(char::is_whitespace) => name,
        name => {
            let error = format!("the ref {name} is not a full ref name");
            return Err(Failure::Failed(error));
        }
    };

    // Fetch the commit or its ref, and detach HEAD on it.
    let depth = format!("--depth={}", checkout.depth);
    let shallow = (checkout.depth > 0).then_some(depth.as_str());
    runner.run(&["init", "--quiet"]).await?;
    runner
        .run(&["remote", "add", "origin", &checkout.clone_url])
        .await?;
    let fetch: Vec<&str> = ["fetch", "--no-tags"]
        .into_iter()
        .chain(shallow)
        .chain(["origin", target])
        .collect();
    runner.run(&fetch).await?;
    runner.run(&["checkout", "--detach", "FETCH_HEAD"]).await?;

    // Check that HEAD is the commit.
    let head = runner.head().await?;
    if head != sha {
        return Err(Failure::Failed(format!(
            "HEAD is {head}, not the commit {sha}"
        )));
    }

    // Get the submodules and the LFS files when the dispatch asks.
    if checkout.submodules {
        let update: Vec<&str> = ["submodule", "update", "--init", "--recursive"]
            .into_iter()
            .chain(shallow)
            .collect();
        runner.run(&update).await?;
    }
    if checkout.lfs {
        runner.run(&["lfs", "pull", "origin"]).await?;
    }
    Ok(())
}

impl Runner<'_> {
    /// Makes a git command in the work folder that never asks for a password. Only `git lfs`
    /// gets the LFS files.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new("git");
        self.config.apply(&mut command);
        command
            .args(args)
            .current_dir(self.dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .stdin(Stdio::null());
        if args.first() != Some(&"lfs") {
            command.env("GIT_LFS_SKIP_SMUDGE", "1");
        }
        command
    }

    /// Runs one git command, with its lines masked in the log of the checkout.
    async fn run(&mut self, args: &[&str]) -> Result<(), Failure> {
        // Tell the command in the log, with no setting.
        let line = output::mask(format!("git {}", args.join(" ")), &self.masks);
        output::report(self.outbox, self.job, NODE, &line).await;

        // Start it, with its output in the log.
        let mut command = self.command(args);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = Process::spawn(command).map_err(|error| {
            Failure::Failed(match error.kind() {
                std::io::ErrorKind::NotFound => {
                    "the agent did not find the program git on its PATH".to_owned()
                }
                _ => format!("the program git did not start: {error}"),
            })
        })?;
        let sink = output::Sink::start(self.outbox.clone(), self.job.to_owned(), NODE.to_owned());
        let masks = self.masks.clone();
        let readers = [
            child
                .stdout()
                .map(|out| output::read(out, LogStream::Stdout, masks.clone(), sink.lines())),
            child
                .stderr()
                .map(|err| output::read(err, LogStream::Stderr, masks, sink.lines())),
        ];

        // Wait for the end, the end of the time, or a stop.
        let deadline = self.deadline;
        let end = tokio::select! {
            status = child.wait() => match status.ok().and_then(|status| status.code()) {
                Some(0) => Ok(()),
                code => Err(Failure::Failed(format!(
                    "git {} ended with exit code {}",
                    args[0],
                    code.map_or("none".to_owned(), |code| code.to_string())
                ))),
            },
            () = sleep_until(deadline) => {
                child.stop().await;
                let error = format!("the checkout ran more than {} seconds", LIMIT.as_secs());
                Err(Failure::Failed(error))
            }
            () = async { let _ = self.stopped.wait_for(|stopped| *stopped).await; } => {
                child.stop().await;
                Err(Failure::Halted)
            }
        };

        // Send the last lines.
        for reader in readers.into_iter().flatten() {
            let _ = reader.await;
        }
        sink.finish().await;
        end
    }

    /// Reads the commit of HEAD.
    async fn head(&self) -> Result<String, Failure> {
        let output = self
            .command(&["rev-parse", "HEAD"])
            .output()
            .await
            .map_err(|error| Failure::Failed(format!("git rev-parse did not start: {error}")))?;
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_ascii_lowercase())
    }
}

/// Returns the values that the log never shows: the header, and its last word, the credential.
fn masks(header: &str) -> Vec<String> {
    let credential = header.rsplit(' ').next().unwrap_or_default();
    [header, credential]
        .into_iter()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}
