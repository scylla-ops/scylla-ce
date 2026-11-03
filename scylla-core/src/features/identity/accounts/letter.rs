//! The layout of the account mails: one text body and one HTML body with inline styles, from
//! the same paragraphs. The HTML body escapes each value.

use scylla_extension::Mail;

/// The style of the body of an HTML mail.
const BODY: &str = "font-family: Helvetica, Arial, sans-serif; color: #1f2328; line-height: 1.5;";

/// The style of the button of a link.
const BUTTON: &str = "display: inline-block; padding: 10px 18px; background: #1f2328; \
                      color: #ffffff; text-decoration: none; border-radius: 6px;";

/// The content of a mail, before the layout.
pub(super) struct Letter<'a> {
    /// The address of the recipient.
    pub(super) to: &'a str,
    /// The subject.
    pub(super) subject: String,
    /// The paragraphs before the link.
    pub(super) intro: Vec<String>,
    /// The link: the text of its button, and its address.
    pub(super) action: Option<(&'a str, String)>,
    /// The paragraphs after the link.
    pub(super) outro: Vec<String>,
}

impl Letter<'_> {
    /// Makes the mail: a text body, and an HTML body with each value escaped.
    pub(super) fn compose(self) -> Mail {
        // Write the text body: the paragraphs, with the link alone on its line.
        let mut text: Vec<String> = vec!["Hello,".to_owned()];
        text.extend(self.intro.iter().cloned());
        if let Some((_, link)) = &self.action {
            text.push(link.clone());
        }
        text.extend(self.outro.iter().cloned());

        // Write the HTML body: the same paragraphs, and a button with a copy of the link.
        let mut html = vec![paragraph("Hello,")];
        html.extend(self.intro.iter().map(|line| paragraph(line)));
        if let Some((label, link)) = &self.action {
            let (label, link) = (escape(label), escape(link));
            html.push(format!(
                r#"<p><a href="{link}" style="{BUTTON}">{label}</a></p>"#
            ));
            html.push(paragraph(
                "If the button does not work, copy this link into your browser:",
            ));
            html.push(format!(r#"<p style="word-break: break-all;">{link}</p>"#));
        }
        html.extend(self.outro.iter().map(|line| paragraph(line)));
        let html = format!(
            "<!DOCTYPE html>\n<html lang=\"en\">\n<body style=\"{BODY}\">\n{}\n</body>\n</html>\n",
            html.join("\n")
        );
        Mail {
            to: self.to.to_owned(),
            subject: self.subject,
            body: text.join("\n\n"),
            html,
        }
    }
}

/// Makes an HTML paragraph of a text.
fn paragraph(text: &str) -> String {
    format!("<p>{}</p>", escape(text))
}

/// Escapes a value for an HTML text node or a quoted attribute.
fn escape(value: &str) -> String {
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

#[cfg(test)]
mod tests {
    //! Unit tests of the layout of the mails.

    use super::{escape, Letter};

    /// Each character that HTML reads as markup is escaped.
    #[test]
    fn escape_replaces_the_markup_characters() {
        // given
        let value = r#"<a href="x">Tom & 'Jerry'</a>"#;

        // when
        let escaped = escape(value);

        // then
        assert_eq!(
            escaped,
            "&lt;a href=&quot;x&quot;&gt;Tom &amp; &#39;Jerry&#39;&lt;/a&gt;"
        );
    }

    /// The text body has the link as it is, and the HTML body has the escaped values and a
    /// button.
    #[test]
    fn compose_writes_a_text_and_an_html_body() {
        // given
        let letter = Letter {
            to: "ann@example.com",
            subject: "Hi".to_owned(),
            intro: vec!["Join <team>.".to_owned()],
            action: Some(("Join", "https://app/x#token=a&b".to_owned())),
            outro: vec!["Bye.".to_owned()],
        };

        // when
        let mail = letter.compose();

        // then
        assert!(mail
            .body
            .contains("Join <team>.\n\nhttps://app/x#token=a&b\n\nBye."));
        assert!(mail.html.contains("<p>Join &lt;team&gt;.</p>"));
        assert!(mail
            .html
            .contains(r#"<a href="https://app/x#token=a&amp;b""#));
        assert!(!mail.html.contains("<team>"));
    }
}
