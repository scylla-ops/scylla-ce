//! The checks of the values of the legal settings: the texts, the addresses and the dates.

use time::{Date, Month};

use super::DocumentValues;
use crate::DomainError;

/// Gives the field of an error.
pub(super) fn at(field: &'static str) -> impl Fn(DomainError) -> (&'static str, DomainError) {
    move |error| (field, error)
}

/// Checks a text of one line: trimmed, at most `max` characters, with no control character.
pub(super) fn line(raw: &str, max: usize) -> Result<String, DomainError> {
    let trimmed = raw.trim();
    if trimmed.chars().count() > max {
        return Err(DomainError::TooLong { max });
    }
    if trimmed.chars().any(char::is_control) {
        return Err(DomainError::NotOneLine);
    }
    Ok(trimmed.to_owned())
}

/// Checks a text of one line or more: trimmed, at most `max` characters, with line breaks and
/// tabs as its only control characters.
pub(super) fn block(raw: &str, max: usize) -> Result<String, DomainError> {
    let trimmed = raw.trim();
    if trimmed.chars().count() > max {
        return Err(DomainError::TooLong { max });
    }
    text_characters(trimmed)?;
    Ok(trimmed.to_owned())
}

/// Checks a text in Markdown: at most `MARKDOWN_MAX` bytes, with line breaks and tabs as its
/// only control characters.
pub(super) fn markdown(raw: String) -> Result<String, DomainError> {
    let max = DocumentValues::MARKDOWN_MAX;
    if raw.len() > max {
        return Err(DomainError::TooLarge { max });
    }
    text_characters(&raw)?;
    Ok(raw)
}

/// Refuses a control character that is not a line break or a tab.
fn text_characters(text: &str) -> Result<(), DomainError> {
    let allowed = |c: char| !c.is_control() || matches!(c, '\n' | '\r' | '\t');
    if text.chars().all(allowed) {
        Ok(())
    } else {
        Err(DomainError::ControlCharacter)
    }
}

/// Checks an http or https address of one line.
pub(super) fn address(raw: &str) -> Result<String, DomainError> {
    let url = line(raw, DocumentValues::URL_MAX)?;
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    match rest {
        Some(rest) if !rest.is_empty() && !rest.contains(' ') => Ok(url),
        _ => Err(DomainError::NotAnUrl),
    }
}

/// Reads a date `YYYY-MM-DD`. An empty text gives `None`.
pub(super) fn date(raw: &str) -> Result<Option<Date>, DomainError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let mut parts = raw.split('-');
    let (Some(year), Some(month), Some(day), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(DomainError::NotADate);
    };
    if year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return Err(DomainError::NotADate);
    }
    // Digits only: `parse` takes a sign too.
    let number = |text: &str| {
        if !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(DomainError::NotADate);
        }
        text.parse::<u16>().map_err(|_| DomainError::NotADate)
    };
    let month = u8::try_from(number(month)?).map_err(|_| DomainError::NotADate)?;
    let month = Month::try_from(month).map_err(|_| DomainError::NotADate)?;
    let day = u8::try_from(number(day)?).map_err(|_| DomainError::NotADate)?;
    Date::from_calendar_date(i32::from(number(year)?), month, day)
        .map(Some)
        .map_err(|_| DomainError::NotADate)
}

/// Writes a date as `YYYY-MM-DD`.
pub fn date_text(date: Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}
