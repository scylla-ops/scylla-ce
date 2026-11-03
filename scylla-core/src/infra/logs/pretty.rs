//! The pretty lines, for a person: one short aligned line for each call, a second line for a
//! refusal or an error, and one line for each event.

use tracing::Level;

use super::mask::short;
use super::record::{Fields, Line, Outcome};

/// The width of the time: `HH:MM:SS.mmm`.
const TIME: usize = 12;
/// The width of the tag column.
const TAG: usize = 10;
/// The width of the route column.
const ROUTE: usize = 36;
/// The width of the duration column.
const DURATION: usize = 7;
/// The width of the actor column.
const ACTOR: usize = 18;

/// A color of the terminal, as its ANSI code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Color {
    /// No color.
    Plain = 0,
    /// Faint text.
    Dim = 2,
    /// Red.
    Red = 31,
    /// Green.
    Green = 32,
    /// Yellow.
    Yellow = 33,
    /// Cyan.
    Cyan = 36,
}

/// The parts of a pretty line.
struct Parts {
    /// The tag: the result of a call, the kind of an event, or a level.
    tag: String,
    /// The color of the tag.
    color: Color,
    /// The text after the tag.
    body: String,
    /// The details of the second line.
    details: Vec<String>,
}

/// Writes a line, and a second line under the route for the details of a call.
pub(super) fn render(line: &Line, time: &str, colors: bool) -> String {
    let parts = match line.event.as_deref() {
        Some("rpc") => call(line),
        Some(name) => event(name, line),
        None => plain(line),
    };
    let paint = |text: &str, color: Color| match colors && color != Color::Plain {
        true => format!("\x1b[{}m{text}\x1b[0m", color as u8),
        false => text.to_owned(),
    };
    let tag = paint(&pad(&parts.tag, TAG), parts.color);
    let mut text = format!("{}  {tag}{}\n", paint(time, Color::Dim), parts.body);
    if !parts.details.is_empty() {
        text.push_str(&" ".repeat(TIME + 2 + TAG));
        text.push_str(&parts.details.join("  "));
        text.push('\n');
    }
    text
}

/// The parts of the line of a call: result, route, duration, actor and IP address.
fn call(line: &Line) -> Parts {
    let call = line.call.clone().unwrap_or_default();
    let (tag, color) = result(&line.fields);
    let route = call.text("rpc").map(|rpc| route(&rpc)).unwrap_or_default();
    let duration = format!("{}ms", line.fields.text("ms").unwrap_or_default());
    let actor = match (call.text("actor_kind"), call.text("actor")) {
        (Some(kind), Some(id)) => format!("{kind} {}", short(&id)),
        _ => "anonymous".to_owned(),
    };
    let ip = call.text("ip").unwrap_or_default();
    let body = format!(
        "{}{duration:>DURATION$}  {}{ip}",
        pad(&route, ROUTE),
        pad(&actor, ACTOR)
    );
    Parts {
        tag,
        color,
        body: body.trim_end().to_owned(),
        details: line.outcome.as_ref().map(details).unwrap_or_default(),
    }
}

/// The parts of the line of a business event: its kind, then its name and its fields. A mail
/// shows its kind, its masked recipient and its result.
fn event(name: &str, line: &Line) -> Parts {
    let tag = match name.split('.').next().unwrap_or_default() {
        "mail" => "MAIL",
        "account" | "password" | "email" | "session" => "AUTH",
        "app" | "agent" => "APP",
        "user" | "admin" => "ADMIN",
        "access" | "role" | "invitation" | "owner" => "ACCESS",
        "workspace" => "WORKSPACE",
        "secret" => "SECRET",
        _ => "EVENT",
    };
    let mut fields = line.fields.clone();
    let (subject, rest) = match name.strip_prefix("mail.") {
        Some(result) => {
            let kind = fields.take("kind").unwrap_or_default();
            let to = fields.take("to").unwrap_or_default();
            let rest = [result.to_owned(), pairs(&fields)].join("  ");
            (format!("{kind} -> {to}"), rest)
        }
        None => (name.to_owned(), pairs(&fields)),
    };
    let color = match line.level {
        Level::ERROR => Color::Red,
        Level::WARN => Color::Yellow,
        _ => Color::Cyan,
    };
    Parts {
        tag: tag.to_owned(),
        color,
        body: format!("{}  {rest}", pad(&subject, ROUTE))
            .trim_end()
            .to_owned(),
        details: Vec::new(),
    }
}

/// The parts of another line: its level, its message and its fields.
fn plain(line: &Line) -> Parts {
    let color = match line.level {
        Level::ERROR => Color::Red,
        Level::WARN => Color::Yellow,
        Level::INFO => Color::Plain,
        _ => Color::Dim,
    };
    let message = line.message.clone().unwrap_or_default();
    let body = [message, pairs(&line.fields)].join("  ");
    Parts {
        tag: line.level.as_str().to_owned(),
        color,
        body: body.trim().to_owned(),
        details: Vec::new(),
    }
}

/// Returns the tag and its color for the gRPC code or the HTTP status of a call.
fn result(fields: &Fields) -> (String, Color) {
    if let Some(code) = fields.text("code") {
        let (tag, color) = match code.as_str() {
            "OK" => ("OK", Color::Green),
            "PERMISSION_DENIED" => ("DENIED", Color::Yellow),
            "FAILED_PRECONDITION" => ("REFUSED", Color::Yellow),
            "INVALID_ARGUMENT" => ("INVALID", Color::Yellow),
            "NOT_FOUND" => ("NOT_FOUND", Color::Yellow),
            "UNAUTHENTICATED" => ("UNAUTH", Color::Yellow),
            "ALREADY_EXISTS" => ("EXISTS", Color::Yellow),
            "RESOURCE_EXHAUSTED" => ("EXHAUSTED", Color::Yellow),
            "ABORTED" => ("ABORTED", Color::Yellow),
            "OUT_OF_RANGE" => ("RANGE", Color::Yellow),
            "UNIMPLEMENTED" => ("UNIMPL", Color::Yellow),
            "CANCELLED" => ("CANCELLED", Color::Yellow),
            "DEADLINE_EXCEEDED" => ("TIMEOUT", Color::Yellow),
            "UNAVAILABLE" => ("UNAVAIL", Color::Red),
            _ => ("ERROR", Color::Red),
        };
        return (tag.to_owned(), color);
    }
    let status = fields
        .text("status")
        .and_then(|text| text.parse::<u16>().ok());
    match status.unwrap_or(500) {
        100..=399 => ("OK".to_owned(), Color::Green),
        400 => ("INVALID".to_owned(), Color::Yellow),
        401 => ("UNAUTH".to_owned(), Color::Yellow),
        403 => ("DENIED".to_owned(), Color::Yellow),
        404 => ("NOT_FOUND".to_owned(), Color::Yellow),
        409 => ("REFUSED".to_owned(), Color::Yellow),
        status @ 400..=499 => (format!("HTTP {status}"), Color::Yellow),
        _ => ("ERROR".to_owned(), Color::Red),
    }
}

/// Returns the details of a call: the refused permission and its object, the reason and the
/// metadata of a refusal, the fields that are not valid, the error, or else the message.
fn details(outcome: &Outcome) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(permission) = &outcome.permission {
        parts.push(format!("permission={permission}"));
    }
    if let Some(object) = &outcome.object {
        parts.push(format!("object={}", short(object)));
    }
    if let Some(reason) = &outcome.reason {
        parts.push(format!("reason={}", quote(reason)));
    }
    for (key, value) in &outcome.metadata {
        parts.push(format!("{key}={}", quote(&short(value))));
    }
    for (field, problem) in &outcome.violations {
        parts.push(format!("{field} {problem}"));
    }
    if let Some(error) = &outcome.error {
        parts.push(format!("error={}", quote(error)));
    }
    if parts.is_empty() {
        parts.extend(outcome.message.clone());
    }
    parts
}

/// Returns the route of a call for a person: `AccountService.SignUp` for a gRPC method, and the
/// method and the path of an HTTP route.
fn route(rpc: &str) -> String {
    match rpc.split_once('/') {
        Some((service, method)) if !rpc.contains(' ') => {
            let service = service.rsplit('.').next().unwrap_or(service);
            format!("{service}.{method}")
        }
        _ => rpc.to_owned(),
    }
}

/// Writes fields as `key=value`, two spaces apart, with the short form of the ids.
fn pairs(fields: &Fields) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{key}={}", quote(&short(&value.to_string()))))
        .collect::<Vec<_>>()
        .join("  ")
}

/// Puts a text in quotes if it is empty or has a space or a quote.
fn quote(text: &str) -> String {
    match text.is_empty() || text.contains(|c: char| c.is_whitespace() || c == '"') {
        true => format!("{text:?}"),
        false => text.to_owned(),
    }
}

/// Pads a text to a width, with one space at least after it.
fn pad(text: &str, width: usize) -> String {
    match text.chars().count() < width {
        true => format!("{text:<width$}"),
        false => format!("{text} "),
    }
}
