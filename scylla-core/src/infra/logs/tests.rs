//! Unit tests of the log lines: the pretty format, the JSON format, and the masks.

use std::io;
use std::sync::{Arc, Mutex};

use scylla_domain::identity::actors::ActorId;
use tracing::field::Empty;

use super::{annotate, capture, mask_email, mask_identifier, LogFormat, Style, CALL};

/// The time of a pretty line: `HH:MM:SS.mmm`.
const TIME: usize = 12;

/// A writer that keeps the log lines in memory.
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

/// Runs `logs` with a capture in a style, and returns the lines.
fn lines_of(style: Style, logs: impl FnOnce()) -> Vec<String> {
    let lines = Lines::default();
    let writer = lines.clone();
    let guard = capture(style, move || writer.clone());
    logs();
    drop(guard);
    let text = String::from_utf8(lines.0.lock().unwrap().clone()).unwrap();
    text.lines().map(str::to_owned).collect()
}

/// Logs a call of a route: an anonymous one, or one of a user that the gate refused.
fn call(route: &str, actor: Option<ActorId>, code: &str, ms: u64) {
    let span = tracing::error_span!(
        CALL,
        request_id = "0193",
        rpc = route,
        ip = "127.0.0.1",
        actor = Empty,
        actor_kind = Empty
    );
    if let Some(actor) = actor {
        span.record("actor", tracing::field::display(actor));
        span.record("actor_kind", "user");
        span.in_scope(|| {
            tracing::warn!(
                event = "access.refused",
                permission = "delete_workspace",
                object = "workspace:0193a5e1-0000-7000-8000-000000000a1f",
                "access refused"
            );
        });
    }
    tracing::info!(parent: &span, event = CALL, code, ms);
}

/// A pretty line of a call is aligned, and a refusal gives one second line with its permission
/// and its object, under the route.
#[test]
fn a_pretty_call_line_is_aligned_with_its_refusal_below() {
    // given
    let user = ActorId::generate();
    let style = Style::Pretty { colors: false };

    // when
    let lines = lines_of(style, || {
        call("scylla.accounts.v1.AccountService/SignUp", None, "OK", 363);
        let route = "scylla.workspaces.v1.WorkspaceService/DeleteWorkspace";
        call(route, Some(user), "PERMISSION_DENIED", 4);
    });

    // then
    assert_eq!(lines.len(), 3, "{lines:#?}");
    assert!(lines[0][TIME..].starts_with("  OK        AccountService.SignUp "));
    assert!(lines[1][TIME..].starts_with("  DENIED    WorkspaceService.DeleteWorkspace "));
    let column = |line: &str, part: &str| line.find(part).unwrap() + part.len();
    assert_eq!(column(&lines[0], "363ms"), column(&lines[1], "4ms"));
    assert_eq!(lines[0].find("anonymous"), lines[1].find("user "));
    let user = user.to_string();
    let short = format!("user ..{}", &user[user.len() - 8..]);
    assert!(lines[1].contains(&short), "{}", lines[1]);
    assert_eq!(
        lines[2],
        format!(
            "{}permission=delete_workspace  object=workspace:..00000a1f",
            " ".repeat(24)
        )
    );
    assert!(lines.iter().all(|line| !line.contains('\x1b')));
}

/// A refusal of a rule shows its reason and its metadata, a bad request its fields, and a
/// failure its error, which has no line of its own.
#[test]
fn a_pretty_call_line_shows_the_reason_and_the_fields() {
    // given
    let style = Style::Pretty { colors: false };
    let span = || tracing::error_span!(CALL, rpc = "s.v1.S/M", actor = Empty, actor_kind = Empty);

    // when
    let lines = lines_of(style, || {
        let refused = span();
        annotate(&refused, |outcome| {
            outcome.reason = Some("LAST_OWNER".to_owned());
            outcome.metadata = vec![("workspace_ids".to_owned(), "a,b".to_owned())];
            outcome.message = Some("a workspace keeps one owner".to_owned());
        });
        tracing::info!(parent: &refused, event = CALL, code = "FAILED_PRECONDITION", ms = 2);
        let invalid = span();
        annotate(&invalid, |outcome| {
            outcome.violations = vec![("email".to_owned(), "must be an email".to_owned())];
        });
        tracing::info!(parent: &invalid, event = CALL, code = "INVALID_ARGUMENT", ms = 1);
        let failed = span();
        failed.in_scope(|| tracing::error!(event = "request.failed", error = "pool timed out"));
        tracing::error!(parent: &failed, event = CALL, code = "INTERNAL", ms = 5);
    });

    // then
    assert_eq!(lines.len(), 6, "{lines:#?}");
    assert!(lines[0].contains("REFUSED   S.M"), "{lines:#?}");
    assert_eq!(lines[1].trim(), "reason=LAST_OWNER  workspace_ids=a,b");
    assert!(lines[2].contains("INVALID   S.M"), "{lines:#?}");
    assert_eq!(lines[3].trim(), "email must be an email");
    assert!(lines[4].contains("ERROR     S.M"), "{lines:#?}");
    assert_eq!(lines[5].trim(), "error=\"pool timed out\"");
}

/// The colors are on the tags only when the style asks for them, and the style follows the
/// terminal and `NO_COLOR`.
#[test]
fn colors_need_a_terminal_and_no_no_color() {
    // given
    let route = "scylla.accounts.v1.AccountService/SignUp";

    // when
    let colored = lines_of(Style::Pretty { colors: true }, || {
        call(route, None, "OK", 1)
    });
    let plain = lines_of(Style::Pretty { colors: false }, || {
        call(route, None, "OK", 1)
    });

    // then
    assert!(
        colored[0].contains("\x1b[32mOK        \x1b[0m"),
        "{colored:?}"
    );
    assert!(!plain[0].contains('\x1b'));
    assert_eq!(
        LogFormat::Auto.style_for(true, false),
        Style::Pretty { colors: true }
    );
    assert_eq!(LogFormat::Auto.style_for(false, false), Style::Json);
    assert_eq!(
        LogFormat::Pretty.style_for(false, false),
        Style::Pretty { colors: false }
    );
    assert_eq!(
        LogFormat::Pretty.style_for(true, true),
        Style::Pretty { colors: false }
    );
}

/// A JSON line of a call has the time in UTC, the level, the event, the route, the code, the
/// duration, the actor, the IP address, the request id and the refusal, in this order.
#[test]
fn a_json_call_line_has_the_fields_of_its_call() {
    // given
    let user = ActorId::generate();

    // when
    let lines = lines_of(Style::Json, || {
        call("s.v1.S/Delete", Some(user), "PERMISSION_DENIED", 4);
    });

    // then
    assert_eq!(lines.len(), 1, "{lines:#?}");
    let line: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    let ts = line["ts"].as_str().unwrap();
    assert!(
        ts.len() == 24 && ts.ends_with('Z') && &ts[19..20] == ".",
        "{ts}"
    );
    assert_eq!(line["level"], "info");
    assert_eq!(line["event"], "rpc");
    assert_eq!(line["rpc"], "s.v1.S/Delete");
    assert_eq!(line["code"], "PERMISSION_DENIED");
    assert_eq!(line["ms"], 4);
    assert_eq!(line["actor"], user.to_string());
    assert_eq!(line["actor_kind"], "user");
    assert_eq!(line["ip"], "127.0.0.1");
    assert_eq!(line["request_id"], "0193");
    assert_eq!(line["permission"], "delete_workspace");
    assert_eq!(
        line["object"],
        "workspace:0193a5e1-0000-7000-8000-000000000a1f"
    );
    let keys = [
        "\"ts\"",
        "\"level\"",
        "\"event\"",
        "\"rpc\"",
        "\"code\"",
        "\"ms\"",
    ];
    let keys = keys
        .iter()
        .chain(&["\"actor\"", "\"ip\"", "\"request_id\""]);
    let places: Vec<_> = keys.map(|key| lines[0].find(key).unwrap()).collect();
    assert!(
        places.windows(2).all(|pair| pair[0] < pair[1]),
        "{places:?}"
    );
}

/// A business event has its own line: a tag and its fields in the pretty format, its name and
/// its fields in JSON. A refusal with no call has its own line too.
#[test]
fn a_business_event_has_its_own_line() {
    // given
    let mail = || {
        tracing::info!(
            event = "mail.sent",
            kind = "verify-email",
            to = "s***@gmail.com"
        )
    };
    let refusal = || tracing::warn!(event = "access.refused", permission = "get_workspace");

    // when
    let pretty = lines_of(Style::Pretty { colors: false }, || {
        mail();
        refusal();
    });
    let json = lines_of(Style::Json, mail);

    // then
    let mail_line = format!("{:<36}  sent", "verify-email -> s***@gmail.com");
    assert_eq!(pretty[0][TIME..], format!("  MAIL      {mail_line}"));
    assert!(pretty[1][TIME..].starts_with("  ACCESS    access.refused "));
    assert!(pretty[1].contains("permission=get_workspace"));
    let line: serde_json::Value = serde_json::from_str(&json[0]).unwrap();
    assert_eq!(line["event"], "mail.sent");
    assert_eq!(line["kind"], "verify-email");
    assert_eq!(line["to"], "s***@gmail.com");
}

/// An email keeps its first character and its domain, and a handle its first character.
#[test]
fn masks_keep_the_first_character_only() {
    // given
    let email = "sebastien@gmail.com";

    // when
    let masked = mask_email(email);

    // then
    assert_eq!(masked, "s***@gmail.com");
    assert_eq!(mask_email("no-at-sign"), "***");
    assert_eq!(mask_identifier(" alice "), "a***");
    assert_eq!(mask_identifier("bob@example.com"), "b***@example.com");
    assert_eq!(
        super::mask::short("project:x,0193a5e1-0000-7000-8000-000000000a1f"),
        "project:x,..00000a1f"
    );
}
