//! The output of a GitHub check run: a title, a Markdown summary with the link of the run and a
//! table of the nodes, and the last log lines of the failed node.

use scylla_domain::integrations::reports::ReportedStatus;
use scylla_domain::projects::jobs::{CheckoutRun, Job, NodeStatus};
use time::OffsetDateTime;

use super::states;

/// The most bytes of the summary and of the text. GitHub takes 65,535 characters, and a
/// character has one byte or more.
pub(super) const MAX_OUTPUT: usize = 65_535;

/// The number of log lines of the failed node in the text.
pub(super) const TAIL_LINES: i64 = 50;

/// The bytes of the text that its words and its fences can take.
const TEXT_FRAME: usize = 512;

/// The output of a check run.
#[derive(Debug)]
pub(super) struct Output {
    /// The title, for example `build failed: 3/4 nodes passed`.
    pub(super) title: String,
    /// The Markdown summary.
    pub(super) summary: String,
    /// The Markdown text with the last log lines of the failed node, if a node failed.
    pub(super) text: Option<String>,
}

/// The last log lines of the failed node of a job.
pub(super) struct Tail {
    /// The node, or `.checkout` for the checkout.
    pub(super) node: String,
    /// The lines, oldest first. The agent masked the secrets in them.
    pub(super) lines: Vec<String>,
}

/// One row of the table: the checkout or a node.
struct Row<'a> {
    /// The name of the node.
    name: &'a str,
    /// The state.
    status: NodeStatus,
    /// The start.
    started_at: Option<OffsetDateTime>,
    /// The end.
    finished_at: Option<OffsetDateTime>,
    /// The attempts, or `None` for the checkout.
    attempts: Option<u32>,
}

/// Returns the node whose log lines the text shows: the checkout or the first node that failed.
pub(super) fn failed_node(job: &Job) -> Option<&str> {
    if job
        .checkout
        .as_ref()
        .is_some_and(|run| run.status == NodeStatus::Failed)
    {
        return Some(CheckoutRun::NODE);
    }
    job.runs
        .iter()
        .find(|run| run.status == NodeStatus::Failed)
        .map(|run| run.node.as_str())
}

/// Returns the output of the check run of a job in the state `status`, with the link `link`
/// of the run in Scylla.
pub(super) fn of(
    name: &str,
    status: ReportedStatus,
    job: &Job,
    link: &str,
    tail: Option<&Tail>,
) -> Output {
    let rows = rows(job);
    Output {
        title: title(name, status, &rows),
        summary: cut(&summary(status, job, link, &rows), MAX_OUTPUT).to_owned(),
        text: tail.and_then(text),
    }
}

/// Returns the rows of the table: the checkout first, then the nodes in their order.
fn rows(job: &Job) -> Vec<Row<'_>> {
    let checkout = job.checkout.iter().map(|run| Row {
        name: "checkout",
        status: run.status,
        started_at: run.started_at,
        finished_at: run.finished_at,
        attempts: None,
    });
    let nodes = job.runs.iter().map(|run| Row {
        name: run.node.as_str(),
        status: run.status,
        started_at: run.started_at,
        finished_at: run.finished_at,
        attempts: (run.attempt > 0).then_some(run.attempt),
    });
    checkout.chain(nodes).collect()
}

/// Returns the title: the state, with the count of the rows that passed.
fn title(name: &str, status: ReportedStatus, rows: &[Row<'_>]) -> String {
    let passed = rows
        .iter()
        .filter(|row| row.status == NodeStatus::Completed)
        .count();
    let count = format!("{passed}/{} nodes passed", rows.len());
    match status {
        ReportedStatus::AwaitingApproval => "Waiting for approval".to_owned(),
        ReportedStatus::Pending => "Queued".to_owned(),
        ReportedStatus::Running => format!("{name} is running: {count}"),
        ReportedStatus::Completed => format!("{name} passed: {count}"),
        ReportedStatus::Failed => format!("{name} failed: {count}"),
        ReportedStatus::Cancelled => format!("{name} was cancelled: {count}"),
        ReportedStatus::Orphaned => format!("{name} failed: the agent was lost"),
        ReportedStatus::InvalidFile => "Invalid pipeline file".to_owned(),
    }
}

/// Returns the summary: the link of the run, a line about the state, then the table of the
/// nodes or the errors of the pipeline file.
fn summary(status: ReportedStatus, job: &Job, link: &str, rows: &[Row<'_>]) -> String {
    // Give the link of the run, and what the state needs.
    let mut summary = format!("[View the run in Scylla]({link})\n\n");
    match status {
        ReportedStatus::AwaitingApproval => {
            summary.push_str("The run comes from a fork. It waits for an approval in Scylla.\n\n");
        }
        ReportedStatus::Orphaned => summary.push_str("The agent of the run went away.\n\n"),
        _ => {}
    }

    // Give the errors of an invalid file. No node ran.
    if status == ReportedStatus::InvalidFile {
        let errors: Vec<String> = job.file_errors.iter().map(ToString::to_string).collect();
        summary.push_str("The pipeline file is not valid, so no node ran.\n\n");
        summary.push_str(&block(&errors.join("\n")));
        return summary;
    }

    // Give the table of the nodes.
    summary.push_str("| Node | State | Duration | Attempts |\n| --- | --- | --- | --- |\n");
    for row in rows {
        let duration = match (row.started_at, row.finished_at) {
            (Some(start), Some(end)) => states::length(end - start),
            _ => String::new(),
        };
        let attempts = row.attempts.map(|count| count.to_string());
        summary.push_str(&format!(
            "| `{}` | {} | {duration} | {} |\n",
            row.name,
            state(row.status),
            attempts.unwrap_or_default(),
        ));
    }
    summary
}

/// Returns the word of the state of a node in the table.
fn state(status: NodeStatus) -> &'static str {
    match status {
        NodeStatus::Pending => "Waiting",
        NodeStatus::Running => "Running",
        NodeStatus::Completed => "✓ Passed",
        NodeStatus::Failed => "✗ Failed",
        NodeStatus::Skipped => "Skipped",
        NodeStatus::Cancelled => "Cancelled",
    }
}

/// Returns the text of the last log lines of a node, cut to the limit, or `None` with no line.
fn text(tail: &Tail) -> Option<String> {
    if tail.lines.is_empty() {
        return None;
    }
    let name = match tail.node.as_str() {
        CheckoutRun::NODE => "checkout",
        node => node,
    };
    let fences = fence(&tail.lines.join("\n")).len() * 2;
    let budget = MAX_OUTPUT.saturating_sub(TEXT_FRAME + fences);
    let lines = last_lines(&tail.lines, budget);
    let header = match lines.len() {
        1 => format!("Last line of `{name}`:\n\n"),
        count => format!("Last {count} lines of `{name}`:\n\n"),
    };
    let text = header + &block(&lines.join("\n"));
    Some(cut(&text, MAX_OUTPUT).to_owned())
}

/// Returns the last lines that fit in `budget` bytes with a line end each. When the last line
/// alone is too long, it keeps its start.
fn last_lines(lines: &[String], budget: usize) -> Vec<&str> {
    let mut kept = Vec::new();
    let mut used = 0;
    for line in lines.iter().rev() {
        if used + line.len() + 1 > budget {
            if kept.is_empty() {
                kept.push(cut(line, budget.saturating_sub(1)));
            }
            break;
        }
        used += line.len() + 1;
        kept.push(line.as_str());
    }
    kept.reverse();
    kept
}

/// Returns a text in a Markdown code block, with a fence longer than each run of backticks in
/// the text.
fn block(text: &str) -> String {
    let fence = fence(text);
    format!("{fence}text\n{text}\n{fence}\n")
}

/// Returns a fence of backticks: three, or one more than the longest run in the text.
fn fence(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for character in text.chars() {
        run = if character == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    "`".repeat((longest + 1).max(3))
}

/// Returns the start of a text that has at most `max` bytes, at the limit of a character.
fn cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
