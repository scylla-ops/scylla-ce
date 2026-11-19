//! The HTML body of an account mail: centered tables with inline styles, which each mail client
//! shows, the style sheet of the dark theme, and a hidden preheader. Each value is escaped.

use super::content::{Action, Badge, Fact, Letter, NotYou, Part, Tone, LOGO_DARK_ID, LOGO_ID};
use super::style::{
    BAND, BORDER, CANVAS, CARD, DISPLAY, FOREGROUND, MONO, MUTED, ON_PRIMARY, PRIMARY, SANS, SHEET,
    WARNING,
};

/// The blank characters after the preheader, so a client shows no text of the body after it.
const FILLER: &str = "&#847;&zwnj;&nbsp;";

/// The start of a table for the layout.
const TABLE: &str =
    r#"<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0""#;

/// Writes the HTML body of a mail.
pub(super) fn body(letter: &Letter<'_>) -> String {
    // Write the card: the intro, the facts and the main link, then the band at its bottom.
    let action = letter.action.as_ref();
    let card = [
        badge(letter.badge),
        title(&letter.title),
        lead(&letter.lead),
        facts(&letter.facts),
        action.map_or_else(String::new, button),
    ]
    .concat();
    let copy = action.filter(|action| action.one_time).map(copy);
    let band = not_you(&letter.not_you) + &copy.unwrap_or_default();

    // Put the logo, the card and the footer in the middle of the canvas.
    let subject = escape(&letter.subject);
    let preheader = escape(&letter.preheader);
    let (to, instance) = (escape(letter.to), escape(letter.instance));
    let footer = format!(
        "{}Sent to {to} by the Scylla instance at {instance}.</p>{}{}</p>",
        small("margin:0;"),
        small("margin:4px 0 0;"),
        escape(&letter.reason),
    );
    let (logo, filler) = (logo(), FILLER.repeat(60));
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"color-scheme\" content=\"light dark\">\n\
         <meta name=\"supported-color-schemes\" content=\"light dark\">\n\
         <title>{subject}</title>\n<style>{SHEET}</style>\n</head>\n\
         <body class=\"canvas\" style=\"margin:0;padding:0;background-color:{CANVAS};\">\n\
         <div style=\"display:none;max-height:0;max-width:0;overflow:hidden;mso-hide:all;\
         opacity:0;font-size:1px;line-height:1px;color:{CANVAS};\">{preheader}{filler}</div>\n\
         {TABLE} class=\"canvas\" style=\"background-color:{CANVAS};\"><tr>\
         <td align=\"center\" style=\"padding:48px 16px;\">\n\
         <!--[if mso]><table role=\"presentation\" width=\"560\" align=\"center\"><tr><td>\
         <![endif]-->\n\
         {TABLE} style=\"width:100%;max-width:560px;\">\n\
         <tr><td align=\"center\" style=\"padding:0 0 24px;\">{logo}</td></tr>\n\
         <tr><td class=\"card\" style=\"background-color:{CARD};border:1px solid {BORDER};\
         border-radius:16px;overflow:hidden;\">{TABLE}>\n\
         <tr><td class=\"pad\" align=\"center\" style=\"padding:40px;text-align:center;\">\
         {TABLE}>{card}</table></td></tr>\n\
         <tr><td class=\"band pad\" align=\"center\" style=\"padding:24px 40px;\
         background-color:{BAND};border-radius:0 0 15px 15px;text-align:center;\">\
         {TABLE}>{band}</table></td></tr>\n\
         </table></td></tr>\n\
         <tr><td class=\"pad\" align=\"center\" style=\"padding:24px 40px 0;text-align:center;\">\
         {footer}</td></tr>\n\
         </table>\n<!--[if mso]></td></tr></table><![endif]-->\n\
         </td></tr></table>\n</body>\n</html>\n"
    )
}

/// Writes one row of a table of the layout, with the style of its cell.
fn row(style: &str, content: &str) -> String {
    format!(r#"<tr><td align="center" style="{style}text-align:center;">{content}</td></tr>"#)
}

/// Writes the two logos: the one with dark letters, and the one with light letters, which only
/// a client in dark mode shows.
fn logo() -> String {
    let image = |class: &str, id: &str| {
        format!(
            "<img class=\"{class}\" src=\"cid:{id}\" width=\"110\" height=\"36\" alt=\"Scylla\" \
             style=\"display:block;margin:0 auto;width:110px;height:36px;border:0;outline:none;\
             text-decoration:none;font-family:{SANS};font-size:24px;font-weight:500;\
             line-height:36px;color:{FOREGROUND};text-align:center;\">"
        )
    };
    format!(
        "{}<!--[if !mso]><!--><div class=\"logo-dark\" style=\"display:none;max-height:0;\
         overflow:hidden;mso-hide:all;\">{}</div><!--<![endif]-->",
        image("logo-light", LOGO_ID),
        image("", LOGO_DARK_ID),
    )
}

/// Writes the badge above the title.
fn badge(badge: Badge) -> String {
    let (class, background, color) = match badge.tone {
        Tone::Neutral => ("neutral", BAND, FOREGROUND),
        Tone::Brand => ("brand", PRIMARY, ON_PRIMARY),
        Tone::Warning => ("", WARNING, FOREGROUND),
    };
    let span = format!(
        "<span class=\"{class}\" style=\"display:inline-block;padding:5px 9px;\
         border-radius:9999px;background-color:{background};color:{color};font-family:{SANS};\
         font-size:12px;font-weight:500;line-height:12px;white-space:nowrap;\">{}</span>",
        escape(badge.label)
    );
    row("padding:0 0 16px;font-size:12px;line-height:22px;", &span)
}

/// Writes the title.
fn title(title: &str) -> String {
    let title = format!(
        "<h1 class=\"fg\" style=\"margin:0;font-family:{DISPLAY};font-size:24px;\
         font-weight:500;line-height:30px;letter-spacing:-0.72px;color:{FOREGROUND};\">{}</h1>",
        escape(title)
    );
    row("padding:0 0 8px;", &title)
}

/// Writes the lead, with its strong values in the color of the title.
fn lead(parts: &[Part]) -> String {
    let text: String = parts
        .iter()
        .map(|part| match part {
            Part::Plain(text) => escape(text),
            Part::Strong(text) => format!(
                "<strong class=\"fg\" style=\"font-weight:500;color:{FOREGROUND};\">{}</strong>",
                escape(text)
            ),
        })
        .collect();
    let style = format!("font-family:{SANS};font-size:16px;line-height:24px;color:{MUTED};");
    row(
        "",
        &format!(r#"<p class="muted" style="margin:0;{style}">{text}</p>"#),
    )
}

/// Writes the facts: a table with a border, one row for each fact, and a line between two rows.
fn facts(facts: &[Fact]) -> String {
    if facts.is_empty() {
        return String::new();
    }
    let rows: String = facts
        .iter()
        .enumerate()
        .map(|(index, fact)| {
            let (class, line) = match index {
                0 => ("", String::new()),
                _ => (" line", format!("border-top:1px solid {BORDER};")),
            };
            let (family, weight) = if fact.code { (MONO, 400) } else { (SANS, 500) };
            format!(
                "<tr><td class=\"muted{class}\" align=\"left\" style=\"{line}padding:12px 16px 12px 0;\
                 font-family:{SANS};font-size:14px;line-height:20px;color:{MUTED};\
                 text-align:left;white-space:nowrap;\">{}</td>\
                 <td class=\"fg{class}\" align=\"right\" style=\"{line}padding:12px 0;\
                 font-family:{family};font-size:14px;font-weight:{weight};line-height:20px;\
                 color:{FOREGROUND};text-align:right;word-break:break-word;\">{}</td></tr>",
                escape(fact.label),
                escape(&fact.value)
            )
        })
        .collect();
    let table = format!(
        "{TABLE} class=\"line\" style=\"border:1px solid {BORDER};border-radius:14px;\
         border-collapse:separate;\"><tr><td style=\"padding:4px 16px;\">{TABLE}>{rows}</table>\
         </td></tr></table>"
    );
    row("padding:32px 0 0;", &table)
}

/// Writes the main button, and the note under it.
fn button(action: &Action) -> String {
    let button = format!(
        "<table role=\"presentation\" align=\"center\" cellpadding=\"0\" cellspacing=\"0\" \
         border=\"0\"><tr><td class=\"button\" align=\"center\" bgcolor=\"{PRIMARY}\" \
         style=\"border-radius:10px;background-color:{PRIMARY};\"><a class=\"button\" \
         href=\"{}\" target=\"_blank\" style=\"display:block;width:240px;padding:8px 0;\
         border-radius:10px;background-color:{PRIMARY};color:{ON_PRIMARY};font-family:{SANS};\
         font-size:14px;font-weight:500;line-height:20px;text-align:center;\
         text-decoration:none;\">{}</a></td></tr></table>",
        escape(&action.url),
        escape(action.label)
    );
    let note = action.note.as_deref().map(|note| {
        let note = format!("{}{}</p>", paragraph("margin:0;"), escape(note));
        row("padding:12px 0 0;", &note)
    });
    row("padding:32px 0 0;", &button) + &note.unwrap_or_default()
}

/// Writes the block for a person who did not ask for the mail, with its outline button.
fn not_you(block: &NotYou) -> String {
    let title = format!(
        "<p class=\"fg\" style=\"margin:0;font-family:{SANS};font-size:14px;font-weight:500;\
         line-height:20px;color:{FOREGROUND};\">{}</p>",
        escape(block.title)
    );
    let text = format!("{}{}</p>", paragraph("margin:0;"), escape(&block.text));
    let link = block.link.as_ref().map(|(label, url)| {
        let button = format!(
            "<table role=\"presentation\" align=\"center\" cellpadding=\"0\" cellspacing=\"0\" \
             border=\"0\"><tr><td class=\"outline\" style=\"border:1px solid {BORDER};\
             border-radius:8px;\"><a class=\"fg\" href=\"{}\" target=\"_blank\" \
             style=\"display:block;padding:5px 10px;font-family:{SANS};font-size:12px;\
             font-weight:500;line-height:16px;color:{FOREGROUND};text-decoration:none;\
             white-space:nowrap;\">{}</a></td></tr></table>",
            escape(url),
            escape(label)
        );
        row("padding:12px 0 0;", &button)
    });
    row("", &title) + &row("padding:4px 0 0;", &text) + &link.unwrap_or_default()
}

/// Writes a line, then the link of the button as text, for a client where the button does not
/// work.
fn copy(action: &Action) -> String {
    let line = format!(
        "{TABLE}><tr><td class=\"line\" style=\"border-top:1px solid {BORDER};font-size:0;\
         line-height:0;\">&nbsp;</td></tr></table>"
    );
    let url = escape(&action.url);
    let hint = format!(
        "{}Button not working? Paste this link into your browser:</p>",
        small("margin:0;")
    );
    let link = format!(
        "<a class=\"link\" href=\"{url}\" target=\"_blank\" style=\"font-family:{MONO};\
         font-size:12px;line-height:16px;color:{PRIMARY};text-decoration:none;\
         word-break:break-all;\">{url}</a>"
    );
    row("padding:16px 0;", &line) + &row("", &hint) + &row("padding:4px 0 0;", &link)
}

/// Opens a paragraph of text: 14 pixels, in the color of the texts.
fn paragraph(margin: &str) -> String {
    format!(
        "<p class=\"muted\" style=\"{margin}font-family:{SANS};font-size:14px;line-height:20px;\
         color:{MUTED};\">"
    )
}

/// Opens a paragraph of small text: 12 pixels, in the color of the texts.
fn small(margin: &str) -> String {
    format!(
        "<p class=\"muted\" style=\"{margin}font-family:{SANS};font-size:12px;line-height:16px;\
         color:{MUTED};\">"
    )
}

/// Escapes a value for an HTML text node or a quoted attribute.
pub(super) fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            c => escaped.push(c),
        }
    }
    escaped
}
