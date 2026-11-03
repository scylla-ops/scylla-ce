//! The layer that writes the log lines. A call is a span named `rpc`, and its line comes at its
//! end. The refusal of the gate and the error of a failed call go on the line of their call,
//! not on their own line.

use std::io::Write;

use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::{LookupSpan, Registry};

use super::format::Style;
use super::record::{Fields, Line, Outcome};
use super::{json, pretty};

/// The name of the span of a call, and of the event of its result.
pub const CALL: &str = "rpc";

/// The event of a refusal of the gate, with its permission, object and reason.
const REFUSAL: &str = "access.refused";

/// The event of a failure of the server, with its error.
const FAILURE: &str = "request.failed";

/// The event of a refused browser request, with its message.
const REFUSED: &str = "request.refused";

/// Writes one line for each event, in a style, to a writer.
pub struct LogLayer<W> {
    /// How the lines look.
    style: Style,
    /// Where the lines go.
    writer: W,
}

impl<W> LogLayer<W> {
    /// Makes the layer.
    pub fn new(style: Style, writer: W) -> Self {
        Self { style, writer }
    }
}

impl<S, W> Layer<S> for LogLayer<W>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'a> MakeWriter<'a> + 'static,
{
    fn on_new_span(&self, attributes: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if attributes.metadata().name() != CALL {
            return;
        }
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut fields = Fields::default();
        attributes.record(&mut fields);
        let mut extensions = span.extensions_mut();
        extensions.insert(fields);
        extensions.insert(Outcome::default());
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut extensions = span.extensions_mut();
        if let Some(fields) = extensions.get_mut::<Fields>() {
            values.record(fields);
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        // Read the fields of the event, and find its call.
        let mut fields = Fields::default();
        event.record(&mut fields);
        let name = fields.take("event");
        let call = ctx
            .event_scope(event)
            .and_then(|scope| scope.from_root().find(|span| span.name() == CALL));

        // Put a refusal or a failure on the line of its call.
        if let Some(call) = &call {
            let mut extensions = call.extensions_mut();
            if let Some(outcome) = extensions.get_mut::<Outcome>() {
                if fold(outcome, name.as_deref(), &fields) {
                    return;
                }
            }
        }

        // Write the line, with the fields of its call and the outcome of a call.
        let extensions = call.as_ref().map(|call| call.extensions());
        let is_result = name.as_deref() == Some(CALL);
        let line = Line {
            level: *event.metadata().level(),
            target: event.metadata().target().to_owned(),
            message: fields.take("message"),
            event: name,
            fields,
            call: extensions
                .as_ref()
                .and_then(|ext| ext.get::<Fields>().cloned()),
            outcome: extensions
                .as_ref()
                .filter(|_| is_result)
                .and_then(|ext| ext.get::<Outcome>().cloned()),
        };
        drop(extensions);
        let text = match self.style {
            Style::Pretty { colors } => {
                let time = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
                pretty::render(&line, &time, colors)
            }
            Style::Json => {
                let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                json::render(&line, &ts)
            }
        };
        let _ = self.writer.make_writer().write_all(text.as_bytes());
    }
}

/// Puts an event on the outcome of its call, and returns `true`, if it is a refusal of the gate
/// or a failure.
fn fold(outcome: &mut Outcome, name: Option<&str>, fields: &Fields) -> bool {
    match name {
        Some(REFUSAL) => {
            outcome.permission = fields.text("permission");
            outcome.object = fields.text("object");
            outcome.reason = fields.text("reason");
        }
        Some(FAILURE) => outcome.error = fields.text("error"),
        Some(REFUSED) => outcome.message = fields.text("message"),
        _ => return false,
    }
    true
}

/// Changes the outcome of a call, for its line. It does nothing with no log layer.
pub fn annotate(span: &tracing::Span, change: impl FnOnce(&mut Outcome)) {
    span.with_subscriber(|(id, dispatch)| {
        let span = dispatch
            .downcast_ref::<Registry>()
            .and_then(|registry| registry.span(id))?;
        let mut extensions = span.extensions_mut();
        extensions.get_mut::<Outcome>().map(change)
    });
}
