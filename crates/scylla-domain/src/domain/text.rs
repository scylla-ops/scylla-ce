//! One newtype for every bounded string in the domain. The kind is a marker
//! implementing [`Rule`]; two markers give two incompatible types.

use crate::domain::errors::{DomainError, DomainResult};
use derive_where::derive_where;
use serde::{Deserialize, Serialize, Serializer};
use std::borrow::Borrow;
use std::fmt;
use std::marker::PhantomData;

pub trait Rule {
    const LABEL: &'static str;
    /// In UTF-8 bytes.
    const MAX: usize;
    const REQUIRED: bool = true;
    /// Keeps control characters other than NUL: scripts, arguments, values.
    const FREE_FORM: bool = false;
    /// Debug and Display print `[REDACTED]`.
    const SECRET: bool = false;

    fn sanitize(raw: String) -> String {
        if raw.trim().len() == raw.len() {
            raw
        } else {
            raw.trim().to_owned()
        }
    }

    /// Runs after the empty, length and control-character checks, which no kind can skip.
    fn check(_: &str) -> DomainResult<()> {
        Ok(())
    }
}

fn bounds<R: Rule>(s: &str) -> DomainResult<()> {
    if R::REQUIRED && s.trim().is_empty() {
        return Err(DomainError::validation(format!(
            "{} cannot be empty",
            R::LABEL
        )));
    }
    if s.len() > R::MAX {
        return Err(DomainError::validation(format!(
            "{} cannot exceed {} bytes",
            R::LABEL,
            R::MAX
        )));
    }
    // NUL fits no Postgres text and no C string, so even free-form kinds refuse it.
    if let Some(c) = s
        .chars()
        .find(|&c| c == '\0' || (!R::FREE_FORM && c.is_control()))
    {
        return Err(DomainError::validation(format!(
            "{} cannot contain control character U+{:04X}",
            R::LABEL,
            c as u32
        )));
    }
    Ok(())
}

pub enum DescriptionRule {}

impl Rule for DescriptionRule {
    const LABEL: &'static str = "Description";
    const MAX: usize = 1024;
    const REQUIRED: bool = false;
    const FREE_FORM: bool = true;
}

pub type Description = Text<DescriptionRule>;

// derive_where, not derive: the marker only lives in the PhantomData and must not need these traits itself.
#[derive(Deserialize)]
#[derive_where(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(try_from = "String", bound = "")]
pub struct Text<R: Rule>(String, PhantomData<R>);

impl<R: Rule> Text<R> {
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let s = R::sanitize(value.into());
        bounds::<R>(&s)?;
        R::check(&s)?;
        Ok(Self(s, PhantomData))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stored text is checked again: a row written under an older rule fails loudly.
impl<R: Rule> TryFrom<String> for Text<R> {
    type Error = DomainError;
    fn try_from(raw: String) -> DomainResult<Self> {
        Self::new(raw)
    }
}

impl<R: Rule> From<Text<R>> for String {
    fn from(text: Text<R>) -> Self {
        text.0
    }
}

const REDACTED: &str = "[REDACTED]";

impl<R: Rule> fmt::Display for Text<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if R::SECRET { REDACTED } else { &self.0 })
    }
}

impl<R: Rule> fmt::Debug for Text<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if R::SECRET {
            f.write_str(REDACTED)
        } else {
            fmt::Debug::fmt(&self.0, f)
        }
    }
}

impl<R: Rule> AsRef<str> for Text<R> {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl<R: Rule> Borrow<str> for Text<R> {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl<R: Rule> Serialize for Text<R> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    enum Label {}
    impl Rule for Label {
        const LABEL: &'static str = "Label";
        const MAX: usize = 5;
    }

    enum Note {}
    impl Rule for Note {
        const LABEL: &'static str = "Note";
        const MAX: usize = 5;
        const REQUIRED: bool = false;
    }

    enum Lower {}
    impl Rule for Lower {
        const LABEL: &'static str = "Lower";
        const MAX: usize = 5;
        fn sanitize(raw: String) -> String {
            raw.trim().to_lowercase()
        }
        fn check(s: &str) -> DomainResult<()> {
            if s.contains(' ') {
                return Err(DomainError::validation("Lower cannot contain spaces"));
            }
            Ok(())
        }
    }

    #[test]
    fn required_trims_then_rejects_empty_and_too_long() {
        assert_eq!(Text::<Label>::new("  abc ").unwrap().as_str(), "abc");
        assert_eq!(
            Text::<Label>::new("   ").unwrap_err().to_string(),
            "Validation failed: Label cannot be empty"
        );
        assert_eq!(
            Text::<Label>::new("abcdef").unwrap_err().to_string(),
            "Validation failed: Label cannot exceed 5 bytes"
        );
    }

    #[test]
    fn optional_accepts_empty() {
        assert_eq!(Text::<Note>::new("  ").unwrap().as_str(), "");
        assert!(Text::<Note>::new("abcdef").is_err());
    }

    #[test]
    fn overrides_run_after_the_bounds_and_cannot_skip_them() {
        assert_eq!(Text::<Lower>::new(" ABC ").unwrap().as_str(), "abc");
        assert!(Text::<Lower>::new("a b").is_err());
        assert!(Text::<Lower>::new("abcdef").is_err());
        assert!(Text::<Lower>::new("").is_err());
    }

    enum Body {}
    impl Rule for Body {
        const LABEL: &'static str = "Body";
        const MAX: usize = 16;
        const FREE_FORM: bool = true;
        fn sanitize(raw: String) -> String {
            raw
        }
    }

    enum Hidden {}
    impl Rule for Hidden {
        const LABEL: &'static str = "Hidden";
        const MAX: usize = 16;
        const SECRET: bool = true;
    }

    #[test]
    fn max_counts_bytes_not_characters() {
        assert!(Text::<Label>::new("éé").is_ok());
        assert!(Text::<Label>::new("ééé").is_err());
        assert!(Text::<Label>::new("éa").is_ok());
    }

    #[test]
    fn one_line_kinds_reject_every_control_character() {
        for bad in ["a\0b", "a\nb", "a\tb", "a\u{1b}b", "a\u{85}b", "a\u{7f}b"] {
            assert!(Text::<Label>::new(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            Text::<Label>::new("a\0b").unwrap_err().to_string(),
            "Validation failed: Label cannot contain control character U+0000"
        );
    }

    #[test]
    fn free_form_kinds_reject_only_nul() {
        assert_eq!(
            Text::<Body>::new("a\n\tb\u{1b}").unwrap().as_str(),
            "a\n\tb\u{1b}"
        );
        assert!(Text::<Body>::new("a\0b").is_err());
    }

    #[test]
    fn untrimmed_kinds_keep_whitespace_but_reject_blank() {
        assert_eq!(Text::<Body>::new(" x\n").unwrap().as_str(), " x\n");
        assert!(Text::<Body>::new(" \n ").is_err());
    }

    #[test]
    fn secret_kinds_never_print_their_value() {
        let text = Text::<Hidden>::new("hunter2").unwrap();
        assert_eq!(format!("{text:?}"), "[REDACTED]");
        assert_eq!(text.to_string(), "[REDACTED]");
        assert_eq!(text.as_str(), "hunter2");
    }

    #[test]
    fn orders_as_its_string() {
        let a = Text::<Label>::new("a").unwrap();
        let b = Text::<Label>::new("b").unwrap();
        assert!(a < b);
    }

    #[test]
    fn serde_round_trips_and_revalidates() {
        let text = Text::<Label>::new("abc").unwrap();
        let json = serde_json::to_string(&text).unwrap();
        assert_eq!(json, "\"abc\"");
        assert_eq!(serde_json::from_str::<Text<Label>>(&json).unwrap(), text);
        assert!(serde_json::from_str::<Text<Label>>("\"abcdef\"").is_err());
    }

    #[test]
    fn behaves_as_a_string_where_a_string_is_asked() {
        let text = Text::<Label>::new("abc").unwrap();
        assert_eq!(text.to_string(), "abc");
        assert_eq!(format!("{text:?}"), "\"abc\"");
        let owned: String = text.clone().into();
        assert_eq!(owned, "abc");
        let set: std::collections::HashSet<Text<Label>> = [text].into_iter().collect();
        assert!(set.contains("abc"));
    }
}
