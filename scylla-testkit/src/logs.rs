//! A capture of the log lines of the server of a test, in the formats of the server.

use std::io;
use std::sync::{Arc, Mutex};

use scylla_core::infra::logs::{self, Style};
use tracing::subscriber::DefaultGuard;

/// The log lines of the thread of a test, from the start of the capture until it drops. The
/// server of a test runs on this thread, so its lines are there.
pub struct Logs {
    /// The text of the lines.
    lines: Lines,
    /// Keeps the capture on until the drop.
    _guard: DefaultGuard,
}

impl Logs {
    /// Starts to capture the pretty lines, with no colors.
    pub fn pretty() -> Self {
        Self::capture(Style::Pretty { colors: false })
    }

    /// Starts to capture the JSON lines.
    pub fn json() -> Self {
        Self::capture(Style::Json)
    }

    /// Starts to capture the lines of a style, with the default filter.
    fn capture(style: Style) -> Self {
        let lines = Lines::default();
        let writer = lines.clone();
        let guard = logs::capture(style, move || writer.clone());
        Self {
            lines,
            _guard: guard,
        }
    }

    /// Returns the text of the lines so far.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.lines.0.lock().expect("log lock")).into_owned()
    }

    /// Returns the lines so far.
    pub fn lines(&self) -> Vec<String> {
        self.text().lines().map(str::to_owned).collect()
    }
}

/// A writer that keeps the text of the lines.
#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<u8>>>);

impl io::Write for Lines {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("log lock").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
