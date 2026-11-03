//! The format of the logs, and how `auto` and the colors follow the terminal.

use std::io::IsTerminal;

/// The format of the logs, from `SCYLLA_LOG_FORMAT`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum LogFormat {
    /// Pretty when stderr is a terminal, else JSON.
    #[default]
    Auto,
    /// One short aligned line for each call, for a person.
    Pretty,
    /// One JSON object for each line, for a machine.
    Json,
}

/// How the lines look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// The pretty lines, with or without colors.
    Pretty {
        /// `true` to color the tags.
        colors: bool,
    },
    /// The JSON lines.
    Json,
}

impl LogFormat {
    /// Returns the style of the lines on stderr. Colors need a terminal and no `NO_COLOR`.
    pub fn style(self) -> Style {
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
        self.style_for(std::io::stderr().is_terminal(), no_color)
    }

    /// Returns the style of the lines for a stream that is a terminal or not.
    pub(super) fn style_for(self, terminal: bool, no_color: bool) -> Style {
        let colors = terminal && !no_color;
        match self {
            Self::Json => Style::Json,
            Self::Pretty => Style::Pretty { colors },
            Self::Auto if terminal => Style::Pretty { colors },
            Self::Auto => Style::Json,
        }
    }
}
