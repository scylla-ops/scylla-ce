//! The labels of the agents: the automatic labels of the host of an agent, the labels that the
//! admins give to an App, and the labels that a pipeline asks for its jobs.

use super::Host;
use crate::DomainError;

/// The automatic label of an agent with a container runtime.
pub const CONTAINER_LABEL: &str = "container";

/// A label of an agent: 1 to 32 lower case letters, digits, dots, dashes or underscores, that
/// starts with a letter or a digit.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AgentLabel(String);

impl AgentLabel {
    /// The most characters of a label.
    pub const MAX_LEN: usize = 32;

    /// Trims a label and checks its form.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let text = raw.trim();
        let allowed =
            |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_');
        let first = text
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
        if first && text.len() <= Self::MAX_LEN && text.chars().all(allowed) {
            Ok(Self(text.to_owned()))
        } else {
            Err(DomainError::NotALabel)
        }
    }

    /// Returns the label as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The labels of an agent or of a pipeline: at most `MAX`, each one time, in their order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentLabels(Vec<AgentLabel>);

impl AgentLabels {
    /// The most labels of an App or of a pipeline.
    pub const MAX: usize = 16;

    /// Checks a list of labels. A refusal gives the index of the bad label, or `None` for a list
    /// that is too long.
    pub fn parse(raw: Vec<String>) -> Result<Self, (Option<usize>, DomainError)> {
        if raw.len() > Self::MAX {
            return Err((None, DomainError::TooMany { max: Self::MAX }));
        }
        let mut labels: Vec<AgentLabel> = Vec::with_capacity(raw.len());
        for (index, text) in raw.iter().enumerate() {
            let label = AgentLabel::parse(text).map_err(|error| (Some(index), error))?;
            if labels.contains(&label) {
                return Err((Some(index), DomainError::Duplicate));
            }
            labels.push(label);
        }
        Ok(Self(labels))
    }

    /// Makes the labels of texts that the server checked before, for example a column of the
    /// database. A text that is not a label, and a second copy of a label, are dropped.
    pub fn of_checked(texts: Vec<String>) -> Self {
        let mut labels: Vec<AgentLabel> = Vec::with_capacity(texts.len());
        for label in texts.iter().filter_map(|text| AgentLabel::parse(text).ok()) {
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        Self(labels)
    }

    /// Returns `true` if the list has no label.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the labels.
    pub fn as_slice(&self) -> &[AgentLabel] {
        &self.0
    }

    /// Returns `true` if the list has the label.
    pub fn contains(&self, label: &AgentLabel) -> bool {
        self.0.contains(label)
    }

    /// Returns the labels as texts, in their order.
    pub fn texts(&self) -> Vec<String> {
        self.0.iter().map(|label| label.0.clone()).collect()
    }
}

impl Host {
    /// Returns the labels that the server gives to an agent from its host: the operating system
    /// (`linux`, `macos`, `windows`), the architecture (`x64`, `arm64`), and `container` with a
    /// container runtime. A value that is not a label gives no label.
    pub fn automatic_labels(&self) -> AgentLabels {
        let os = self.os.to_ascii_lowercase();
        let os = match os.as_str() {
            "darwin" | "osx" => "macos".to_owned(),
            _ => os,
        };
        let arch = self.arch.to_ascii_lowercase();
        let arch = match arch.as_str() {
            "x86_64" | "amd64" => "x64".to_owned(),
            "aarch64" => "arm64".to_owned(),
            _ => arch,
        };
        let container = self
            .container_runtime
            .as_ref()
            .map(|_| CONTAINER_LABEL.to_owned());
        AgentLabels::of_checked(
            [Some(os), Some(arch), container]
                .into_iter()
                .flatten()
                .collect(),
        )
    }
}

/// Returns `true` if an agent with these automatic labels and these labels of the admins has
/// each label that a job asks.
pub fn fits(asked: &AgentLabels, automatic: &AgentLabels, given: &AgentLabels) -> bool {
    asked
        .as_slice()
        .iter()
        .all(|label| automatic.contains(label) || given.contains(label))
}
