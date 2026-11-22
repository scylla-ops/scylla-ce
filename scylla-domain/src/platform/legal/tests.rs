//! The tests of the legal settings: the dates, the addresses and the kinds.

use super::checks::{address, date};
use super::*;

/// A date reads as `YYYY-MM-DD`, and an empty text is no date.
#[test]
fn date_reads_an_iso_day() {
    let day = date("2027-01-31").unwrap().unwrap();
    assert_eq!(date_text(day), "2027-01-31");
    assert_eq!(date(" ").unwrap(), None);
    for bad in [
        "2027-02-30",
        "2027-1-31",
        "31/01/2027",
        "2027-01-31-01",
        "+027-01-31",
    ] {
        assert_eq!(date(bad), Err(DomainError::NotADate), "{bad}");
    }
}

/// An address must be http or https, with a host.
#[test]
fn address_needs_http() {
    assert!(address("https://example.com/privacy").is_ok());
    for bad in [
        "ftp://example.com",
        "https://",
        "javascript:alert(1)",
        "https://a b",
    ] {
        assert_eq!(address(bad), Err(DomainError::NotAnUrl), "{bad}");
    }
}

/// Each kind reads back from its name.
#[test]
fn kind_reads_its_name() {
    for kind in DocumentKind::ALL {
        assert_eq!(DocumentKind::parse(kind.as_str()), Some(kind));
    }
    assert_eq!(DocumentKind::parse("other"), None);
}
