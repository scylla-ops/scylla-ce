//! The text body of an account mail, for a client with no HTML: the same content as the HTML
//! body, with each link alone after its label.

use super::content::{Letter, Part};
use super::legal;

/// Writes the text body of a mail: blocks with a blank line between two of them.
pub(super) fn body(letter: &Letter) -> String {
    // Write the title, the lead and the facts.
    let lead: String = letter
        .lead
        .iter()
        .map(|part| match part {
            Part::Plain(text) | Part::Strong(text) => text.as_str(),
        })
        .collect();
    let mut blocks = vec![letter.title.clone(), lead];
    if !letter.facts.is_empty() {
        let facts = letter.facts.iter().map(|fact| {
            let (label, value) = (fact.label, &fact.value);
            format!("{label}: {value}")
        });
        blocks.push(facts.collect::<Vec<_>>().join("\n"));
    }

    // Write the main link with its note, then the block for a person who did not ask for it.
    if let Some(action) = &letter.action {
        let mut block = format!("{}: {}", action.label, action.url);
        if let Some(note) = &action.note {
            block = format!("{block}\n{note}");
        }
        blocks.push(block);
    }
    let not_you = &letter.not_you;
    let mut block = format!("{}\n{}", not_you.title, not_you.text);
    if let Some((label, url)) = &not_you.link {
        block = format!("{block}\n{label}: {url}");
    }
    blocks.push(block);

    // Write the footer, with the legal links of the instance.
    blocks.push(format!(
        "--\nSent to {} by the Scylla instance at {}.\n{}{}",
        letter.to,
        letter.instance,
        letter.reason,
        legal::text(&letter.legal)
    ));
    blocks.join("\n\n") + "\n"
}
