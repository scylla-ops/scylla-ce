//! The logs of a job: batches of lines of one node, as the agent sends them.

use time::OffsetDateTime;

use crate::Id;

/// The marker of the id of a batch of lines. The id is also the cursor of a tail.
pub enum LogBatchKey {}

/// The id of a batch of lines.
pub type LogBatchId = Id<LogBatchKey>;

/// The output of a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogStream {
    /// The standard output.
    Stdout,
    /// The standard error.
    Stderr,
}

/// A line of the output of a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// The output.
    pub stream: LogStream,
    /// The text, with no line break at its end.
    pub text: String,
    /// The time when the agent read the line.
    pub at: OffsetDateTime,
}

impl LogLine {
    /// The longest text of a line, in bytes. A longer text is cut.
    pub const MAX_TEXT: usize = 64 << 10;
    /// The most lines in one batch.
    pub const MAX_BATCH: usize = 1000;

    /// Cuts a text at `MAX_TEXT` bytes, on a character boundary.
    pub fn cut(mut text: String) -> String {
        if text.len() > Self::MAX_TEXT {
            let mut end = Self::MAX_TEXT;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        text
    }
}

/// A batch of lines of one node, as the store returns it.
#[derive(Clone, Debug)]
pub struct LogBatch {
    /// The id of the batch: the cursor after it.
    pub id: LogBatchId,
    /// The node.
    pub node: String,
    /// The lines, in order.
    pub lines: Vec<LogLine>,
}
