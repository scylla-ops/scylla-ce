//! The masks of the personal data in the logs, and the short form of the ids in the pretty
//! lines.

use scylla_domain::Id;

/// The mark of the hidden characters.
const HIDDEN: &str = "***";

/// Masks an email: `sebastien@gmail.com` gives `s***@gmail.com`. A text with no `@` gives
/// `***`.
pub fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => format!("{}{HIDDEN}@{domain}", first(local)),
        None => HIDDEN.to_owned(),
    }
}

/// Masks the identifier of a sign-in: an email as `mask_email` does, and a handle to its first
/// character: `alice` gives `a***`.
pub fn mask_identifier(identifier: &str) -> String {
    let identifier = identifier.trim();
    match identifier.contains('@') {
        true => mask_email(identifier),
        false => format!("{}{HIDDEN}", first(identifier)),
    }
}

/// Returns the short form of each id in a text, for a person: `workspace:..00000a1f`. It keeps
/// the end of the id, which is random: the start of a UUIDv7 is the time, the same for recent
/// ids. The parts of the text are between `:` and `,`, and a part that is not an id stays.
pub(super) fn short(text: &str) -> String {
    let mut shown = String::with_capacity(text.len());
    for piece in text.split_inclusive([':', ',']) {
        let part = piece.trim_end_matches([':', ',']);
        match Id::<()>::parse(part) {
            Ok(_) => {
                let chars: Vec<char> = part.chars().collect();
                shown.push_str("..");
                shown.extend(&chars[chars.len() - 8..]);
                shown.push_str(&piece[part.len()..]);
            }
            Err(_) => shown.push_str(piece),
        }
    }
    shown
}

/// Returns the first character of a text, or nothing.
fn first(text: &str) -> String {
    text.chars().take(1).collect()
}
