//! The end of the footer of an account mail: the legal links of the instance and its operator.

use super::html::{escape, small};
use super::style::{MUTED, SANS};

/// The legal links and the operator of the instance. Empty: the footer has no legal part.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Legal {
    /// The links: their text and their address.
    pub(crate) links: Vec<(&'static str, String)>,
    /// The name and the address of the operator, on one line. Empty: none.
    pub(crate) operator: String,
}

/// Writes the legal part of the text footer: one line for each link, then the operator.
pub(super) fn text(legal: &Legal) -> String {
    let links = legal
        .links
        .iter()
        .map(|(label, url)| format!("\n{label}: {url}"));
    let operator = Some(&legal.operator)
        .filter(|operator| !operator.is_empty())
        .map(|operator| format!("\n{operator}"));
    links.chain(operator).collect()
}

/// Writes the legal part of the HTML footer: the links on one line, then the operator.
pub(super) fn html(legal: &Legal) -> String {
    let mut html = String::new();
    if !legal.links.is_empty() {
        let links: Vec<String> = legal
            .links
            .iter()
            .map(|(label, url)| {
                format!(
                    "<a class=\"muted\" href=\"{}\" target=\"_blank\" style=\"font-family:{SANS};\
                     color:{MUTED};text-decoration:underline;\">{}</a>",
                    escape(url),
                    escape(label)
                )
            })
            .collect();
        html.push_str(&format!(
            "{}{}</p>",
            small("margin:12px 0 0;"),
            links.join(" &middot; ")
        ));
    }
    if !legal.operator.is_empty() {
        html.push_str(&format!(
            "{}{}</p>",
            small("margin:4px 0 0;"),
            escape(&legal.operator)
        ));
    }
    html
}
