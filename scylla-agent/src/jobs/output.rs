//! The output of an attempt: each line, with the values of the secrets masked, goes to the
//! server in batches.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use prost_types::Timestamp;
use scylla_proto::pipelines::v1::{LogLine, LogStream};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::events::{self, Outbox};

/// The most lines in one batch.
const BATCH: usize = 500;

/// The longest wait before a batch goes.
const FLUSH: Duration = Duration::from_millis(200);

/// The longest line, in bytes.
const MAX_LINE: usize = 64 << 10;

/// The shortest value that the agent masks.
const MIN_MASK: usize = 4;

/// The batches of the lines of one attempt.
pub(super) struct Sink {
    /// The lines to batch.
    lines: Option<mpsc::Sender<LogLine>>,
    /// The task that sends the batches.
    task: JoinHandle<()>,
}

impl Sink {
    /// Starts the task that batches the lines of a node of a job.
    pub(super) fn start(outbox: Outbox, job: String, node: String) -> Self {
        let (lines, mut input) = mpsc::channel::<LogLine>(BATCH * 2);
        let task = tokio::spawn(async move {
            let mut batch = Vec::new();
            loop {
                // Add the lines until the batch is full or the wait ends.
                let line = if batch.is_empty() {
                    input.recv().await
                } else {
                    match tokio::time::timeout(FLUSH, input.recv()).await {
                        Ok(line) => line,
                        Err(_) => {
                            events::logs(&outbox, &job, &node, std::mem::take(&mut batch)).await;
                            continue;
                        }
                    }
                };
                let Some(line) = line else { break };
                batch.push(line);
                if batch.len() >= BATCH {
                    events::logs(&outbox, &job, &node, std::mem::take(&mut batch)).await;
                }
            }

            // Send the last lines.
            if !batch.is_empty() {
                events::logs(&outbox, &job, &node, batch).await;
            }
        });
        Self {
            lines: Some(lines),
            task,
        }
    }

    /// Returns a sender of lines.
    pub(super) fn lines(&self) -> mpsc::Sender<LogLine> {
        self.lines.clone().expect("a sink that did not finish")
    }

    /// Sends the last batch, and ends the task.
    pub(super) async fn finish(mut self) {
        self.lines.take();
        let _ = self.task.await;
    }
}

/// Sends a line of the agent itself in the logs of a node, because a process that does not start
/// writes nothing.
pub(super) async fn report(outbox: &Outbox, job: &str, node: &str, text: &str) {
    let line = LogLine {
        stream: LogStream::Stderr.into(),
        text: cut(format!("scylla-agent: {text}")),
        at: Some(now()),
    };
    events::logs(outbox, job, node, vec![line]).await;
}

/// Reads the lines of a pipe until its end, and sends them masked.
pub(super) fn read<R>(
    pipe: R,
    stream: LogStream,
    masks: Vec<String>,
    sink: mpsc::Sender<LogLine>,
) -> JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(pipe).lines();
        while let Ok(Some(text)) = lines.next_line().await {
            let line = LogLine {
                stream: stream.into(),
                text: cut(mask(text, &masks)),
                at: Some(now()),
            };
            if sink.send(line).await.is_err() {
                break;
            }
        }
    })
}

/// Replaces each value of a secret of 4 characters or more with `***`.
pub(super) fn mask(mut text: String, masks: &[String]) -> String {
    for value in masks.iter().filter(|value| value.len() >= MIN_MASK) {
        if text.contains(value.as_str()) {
            text = text.replace(value.as_str(), "***");
        }
    }
    text
}

/// Cuts a line at 64 KiB, on a character boundary.
fn cut(mut text: String) -> String {
    if text.len() > MAX_LINE {
        let mut end = MAX_LINE;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// Returns the time now.
fn now() -> Timestamp {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp {
        seconds: since.as_secs() as i64,
        nanos: since.subsec_nanos() as i32,
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests of the masks of the lines.

    use super::mask;

    /// A secret goes, and a short value stays.
    #[test]
    fn a_secret_is_masked() {
        // given
        let masks = vec!["s3cr3t".to_owned(), "ab".to_owned()];

        // when
        let line = mask("token s3cr3t and ab".to_owned(), &masks);

        // then
        assert_eq!(line, "token *** and ab");
    }
}
