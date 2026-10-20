use std::borrow::Cow;
use std::io;
use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use tokio::process::{ChildStderr, ChildStdout};
use tokio_stream::StreamExt;
use tokio_util::bytes::BytesMut;
use tokio_util::codec::{Decoder, FramedRead};

use scylla_domain::domain::job::LogStream;
use scylla_proto::agent::MAX_LOG_LINE_BYTES;
use scylla_proto::agent::v1::{JobLogLine, agent_up};
use scylla_proto::common::v1 as common;

use crate::reporter::StatusPublisher;

const MIN_MASKED_CHARS: usize = 4;
const MASK: &str = "***";

#[derive(Debug, Default)]
pub struct Secrets(Vec<String>);

impl Secrets {
    #[must_use]
    pub fn new(values: impl IntoIterator<Item = String>) -> Self {
        let mut pieces = Vec::new();
        for value in values {
            pieces.extend(
                value
                    .lines()
                    .map(str::trim)
                    .filter(|piece| piece.chars().count() >= MIN_MASKED_CHARS)
                    .map(str::to_owned),
            );
        }
        pieces.sort_unstable_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        pieces.dedup();
        Self(pieces)
    }

    #[must_use]
    pub fn redact(&self, line: &str) -> String {
        self.0.iter().fold(line.to_owned(), |line, secret| {
            line.replace(secret.as_str(), MASK)
        })
    }
}

pub struct NodeLog {
    publisher: StatusPublisher,
    node_id: String,
    secrets: Arc<Secrets>,
    last: DateTime<Utc>,
}

impl NodeLog {
    #[must_use]
    pub fn new(publisher: StatusPublisher, node_id: String, secrets: Arc<Secrets>) -> Self {
        Self {
            publisher,
            node_id,
            secrets,
            last: DateTime::<Utc>::MIN_UTC,
        }
    }

    #[must_use]
    pub fn job_id(&self) -> &str {
        self.publisher.job_id()
    }

    pub async fn send(&mut self, stream: LogStream, text: &str) {
        let at = Utc::now().max(self.last + TimeDelta::microseconds(1));
        self.last = at;
        let line = JobLogLine {
            job_id: Some(common::JobId {
                value: self.job_id().to_owned(),
            }),
            node_id: Some(common::NodeId {
                value: self.node_id.clone(),
            }),
            stream: scylla_proto::convert::log_stream_to_proto(stream) as i32,
            line: self.secrets.redact(text),
            timestamp: scylla_proto::convert::timestamp(at),
        };
        let _ = self.publisher.send(agent_up::Payload::Log(line)).await;
    }
}

pub async fn forward(mut log: NodeLog, stdout: ChildStdout, stderr: ChildStderr) {
    let mut lines = FramedRead::new(stdout, Lines)
        .map(|line| (LogStream::Stdout, line))
        .merge(FramedRead::new(stderr, Lines).map(|line| (LogStream::Stderr, line)));
    while let Some((stream, line)) = lines.next().await {
        if let Ok(line) = line {
            log.send(stream, &text(&line)).await;
        }
    }
}

fn text(line: &[u8]) -> Cow<'_, str> {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    String::from_utf8_lossy(line)
}

struct Lines;

impl Decoder for Lines {
    type Item = BytesMut;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<BytesMut>> {
        let window = &buf[..buf.len().min(MAX_LOG_LINE_BYTES + 1)];
        if let Some(end) = window.iter().position(|&b| b == b'\n') {
            return Ok(Some(buf.split_to(end + 1)));
        }
        if buf.len() <= MAX_LOG_LINE_BYTES {
            return Ok(None);
        }
        let cut = (MAX_LOG_LINE_BYTES - 3..=MAX_LOG_LINE_BYTES)
            .rev()
            .find(|&i| buf[i] & 0xC0 != 0x80)
            .unwrap_or(MAX_LOG_LINE_BYTES);
        Ok(Some(buf.split_to(cut)))
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> io::Result<Option<BytesMut>> {
        match self.decode(buf)? {
            None if !buf.is_empty() => Ok(Some(buf.split())),
            line => Ok(line),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(input: &[u8]) -> Vec<BytesMut> {
        let mut buf = BytesMut::from(input);
        let mut lines = Vec::new();
        while let Some(line) = Lines.decode_eof(&mut buf).unwrap() {
            lines.push(line);
        }
        lines
    }

    fn texts(input: &[u8]) -> Vec<String> {
        decode_all(input)
            .iter()
            .map(|l| text(l).into_owned())
            .collect()
    }

    #[test]
    fn lines_drop_their_terminator_and_keep_a_partial_last_line() {
        assert_eq!(texts(b"a\nb\r\nc"), ["a", "b", "c"]);
        assert_eq!(texts(b"\n"), [""]);
        assert!(texts(b"").is_empty());
    }

    #[test]
    fn a_long_line_is_split_without_losing_bytes() {
        let input = vec![b'x'; 200_000];
        let chunks = decode_all(&input);
        assert!(chunks.iter().all(|c| c.len() <= MAX_LOG_LINE_BYTES));
        assert_eq!(chunks.concat(), input);
    }

    #[test]
    fn a_line_of_exactly_the_limit_is_one_line() {
        let mut input = vec![b'x'; MAX_LOG_LINE_BYTES];
        input.push(b'\n');
        let lines = texts(&input);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), MAX_LOG_LINE_BYTES);
    }

    #[test]
    fn a_split_never_cuts_a_multibyte_character() {
        let input = "é".repeat(100_000);
        let chunks = decode_all(input.as_bytes());
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= MAX_LOG_LINE_BYTES);
            assert!(std::str::from_utf8(chunk).is_ok());
        }
        assert_eq!(chunks.concat(), input.as_bytes());
    }

    #[test]
    fn invalid_utf8_becomes_the_replacement_character() {
        assert_eq!(text(b"\xff\xfebad\n"), "\u{FFFD}\u{FFFD}bad");
    }

    #[test]
    fn a_trailing_newline_or_carriage_return_does_not_defeat_masking() {
        let secrets = Secrets::new(["trail-secret-777\n".into(), "crlf-secret-888\r".into()]);
        assert_eq!(
            secrets.redact("t=trail-secret-777 c=crlf-secret-888"),
            "t=*** c=***"
        );
    }

    #[test]
    fn every_line_of_a_multiline_value_is_masked() {
        let secrets = Secrets::new(["line-one-AAA\nline-two-BBB\r\nline-three-CCC".into()]);
        assert_eq!(secrets.redact("multi=[line-one-AAA"), "multi=[***");
        assert_eq!(secrets.redact("line-two-BBB"), "***");
        assert_eq!(secrets.redact("line-three-CCC]"), "***]");
    }

    #[test]
    fn the_longest_value_is_masked_first_whatever_the_order() {
        for values in [["abc1", "abc1def-longer"], ["abc1def-longer", "abc1"]] {
            let secrets = Secrets::new(values.map(String::from));
            assert_eq!(secrets.redact("x=abc1def-longer"), "x=***");
        }
    }

    #[test]
    fn short_blank_and_duplicate_values_are_ignored() {
        let secrets = Secrets::new(["e".into(), "   \n\t".into(), String::new()]);
        assert!(secrets.0.is_empty());
        assert_eq!(secrets.redact("hello there"), "hello there");

        let secrets = Secrets::new(["s3cr3t".into(), "s3cr3t".into()]);
        assert_eq!(secrets.0, ["s3cr3t"]);
        assert_eq!(secrets.redact("a s3cr3t b s3cr3t"), "a *** b ***");
    }
}
