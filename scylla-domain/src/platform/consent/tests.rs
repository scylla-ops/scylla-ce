//! The tests of the records of the consent: the purposes, the hash and the choices.

use super::*;

/// A SHA-256 of a text, as the web app sends it.
const HASH: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

/// Makes the choice of a purpose.
fn choice(purpose: &str, granted: bool) -> Choice {
    Choice {
        purpose: Purpose::parse(purpose).unwrap(),
        granted,
    }
}

/// Checks a record with these choices and this action.
fn record(
    choices: Vec<Choice>,
    action: ConsentAction,
) -> Result<NewConsentRecord, (&'static str, DomainError)> {
    NewConsentRecord::new(
        None,
        "3fa1c2d0",
        HASH,
        choices,
        action,
        ConsentSource::Banner,
    )
}

/// A purpose is 1 to 32 lower case letters, digits and dashes.
#[test]
fn purpose_is_a_short_lower_case_name() {
    assert!(Purpose::parse("analytics").is_ok());
    assert!(Purpose::parse("ad-2").is_ok());
    for bad in ["", "Analytics", "ad_2", "a b", &"a".repeat(33)] {
        assert_eq!(Purpose::parse(bad), Err(DomainError::NotAPurpose), "{bad}");
    }
}

/// The hash is a SHA-256 in lower case hexadecimal, and the version is one line.
#[test]
fn new_checks_the_version_and_the_hash() {
    let choices = || vec![choice("analytics", true)];
    let bad_hash = NewConsentRecord::new(
        None,
        "1",
        &HASH.to_uppercase(),
        choices(),
        ConsentAction::AcceptAll,
        ConsentSource::Banner,
    );
    assert_eq!(bad_hash, Err(("banner_hash", DomainError::NotASha256)));
    for (bad, error) in [
        ("", DomainError::Empty),
        ("a\nb", DomainError::NotOneLine),
        (&"v".repeat(65), DomainError::TooLong { max: 64 }),
    ] {
        let result = NewConsentRecord::new(
            None,
            bad,
            HASH,
            choices(),
            ConsentAction::AcceptAll,
            ConsentSource::Banner,
        );
        assert_eq!(result, Err(("banner_version", error)));
    }
}

/// A record has 1 to 16 choices, each purpose once.
#[test]
fn new_needs_one_choice_for_each_purpose() {
    let custom = ConsentAction::Custom;
    assert_eq!(record(vec![], custom), Err(("choices", DomainError::Empty)));
    let twice = vec![choice("analytics", true), choice("analytics", false)];
    assert_eq!(
        record(twice, custom),
        Err(("choices", DomainError::Duplicate))
    );
    let many = (0..17).map(|n| choice(&format!("p{n}"), false)).collect();
    assert_eq!(
        record(many, custom),
        Err(("choices", DomainError::TooMany { max: 16 }))
    );
}

/// Accept all accepts each purpose, and reject all refuses each purpose.
#[test]
fn new_needs_choices_that_fit_the_action() {
    let mixed = || vec![choice("analytics", true), choice("support", false)];
    for action in [ConsentAction::AcceptAll, ConsentAction::RejectAll] {
        assert_eq!(
            record(mixed(), action),
            Err(("choices", DomainError::NotTheAction))
        );
    }
    assert!(record(mixed(), ConsentAction::Custom).is_ok());
    assert!(record(mixed(), ConsentAction::Withdraw).is_ok());
    assert!(record(vec![choice("support", false)], ConsentAction::RejectAll).is_ok());
}
