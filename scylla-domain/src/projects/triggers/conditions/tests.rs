//! Unit tests of the checks and of the matches of the conditions of a webhook.

use serde_json::{json, Value};

use super::{Condition, ConditionOperator, ConditionSource, Delivery, WebhookRule, MAX_CONDITIONS};
use crate::DomainError;

/// Makes a condition on a header.
fn header(name: &str, operator: ConditionOperator, values: &[&str]) -> Condition {
    Condition {
        source: ConditionSource::Header(name.to_owned()),
        operator,
        values: values.iter().map(|value| (*value).to_owned()).collect(),
    }
}

/// Makes a condition on a JSON pointer.
fn field(pointer: &str, operator: ConditionOperator, values: &[&str]) -> Condition {
    Condition {
        source: ConditionSource::Pointer(pointer.to_owned()),
        operator,
        values: values.iter().map(|value| (*value).to_owned()).collect(),
    }
}

/// Returns the reason of a skip of a delivery with the header `x-github-event: push` and a body.
fn skip(conditions: Vec<Condition>, body: Option<&Value>) -> Option<String> {
    let headers = |name: &str| (name == "x-github-event").then(|| "push".to_owned());
    let delivery = Delivery {
        header: &headers,
        body,
    };
    WebhookRule { conditions }.skip_reason(&delivery)
}

/// Returns the path and the rule of the refusal of one condition.
fn refusal(condition: Condition) -> (String, DomainError) {
    let rule = WebhookRule {
        conditions: vec![condition],
    };
    rule.check("webhook").unwrap_err()
}

/// All the conditions must match, and a header name is the same in each case.
#[test]
fn a_delivery_that_matches_each_condition_is_not_skipped() {
    // given
    let body =
        json!({ "ref": "refs/heads/release/1.2", "repository": { "full_name": "acme/api" } });
    let conditions = vec![
        header("X-GitHub-Event", ConditionOperator::Is, &["push"]),
        field(
            "/ref",
            ConditionOperator::Matches,
            &["refs/heads/release/*"],
        ),
        field(
            "/repository/full_name",
            ConditionOperator::OneOf,
            &["acme/api", "acme/web"],
        ),
    ];

    // when
    let reason = skip(conditions, Some(&body));

    // then
    assert_eq!(reason, None);
}

/// The first condition that does not match gives the reason, with the value of the delivery.
#[test]
fn a_delivery_that_misses_a_condition_is_skipped_with_the_reason() {
    // given
    let body = json!({ "ref": "refs/heads/dev" });
    let conditions = vec![
        header("x-github-event", ConditionOperator::Is, &["push"]),
        field("/ref", ConditionOperator::Is, &["refs/heads/main"]),
    ];

    // when
    let reason = skip(conditions, Some(&body));

    // then
    assert_eq!(reason.as_deref(), Some("No match: /ref is refs/heads/dev"));
}

/// A glob of a match stops at a `/`, as a glob of the branches, and `**` does not.
#[test]
fn a_glob_star_stops_at_a_slash() {
    let body = json!({ "ref": "refs/heads/release/1/2" });
    let one = vec![field(
        "/ref",
        ConditionOperator::Matches,
        &["refs/heads/release/*"],
    )];
    let two = vec![field(
        "/ref",
        ConditionOperator::Matches,
        &["refs/heads/**"],
    )];

    assert!(skip(one, Some(&body)).is_some());
    assert_eq!(skip(two, Some(&body)), None);
}

/// A missing header, a missing value and a body that is not JSON do not match.
#[test]
fn a_missing_value_does_not_match() {
    let body = json!({ "after": "abc", "count": 3 });
    let missing_header = vec![header(
        "X-Gitlab-Event",
        ConditionOperator::Is,
        &["Push Hook"],
    )];
    let missing_field = vec![field("/ref", ConditionOperator::Is, &["main"])];
    let not_json = vec![field("/after", ConditionOperator::Is, &["abc"])];
    let number = vec![field("/count", ConditionOperator::Is, &["3"])];

    assert_eq!(
        skip(missing_header, Some(&body)).as_deref(),
        Some("No match: X-Gitlab-Event is missing")
    );
    assert_eq!(
        skip(missing_field, Some(&body)).as_deref(),
        Some("No match: /ref is missing")
    );
    assert_eq!(
        skip(not_json, None).as_deref(),
        Some("No match: the body is not JSON")
    );
    assert_eq!(skip(number, Some(&body)), None);
}

/// A rule with no condition matches each delivery.
#[test]
fn no_condition_matches_each_delivery() {
    assert_eq!(skip(Vec::new(), None), None);
}

/// The check refuses each bad condition on its field.
#[test]
fn the_check_refuses_a_bad_condition_on_its_field() {
    let is = ConditionOperator::Is;
    let one_of = ConditionOperator::OneOf;
    let cases = [
        (header(" ", is, &["push"]), "header", DomainError::Empty),
        (
            header("X Event", is, &["push"]),
            "header",
            DomainError::NotAHeaderName,
        ),
        (
            field("ref", is, &["main"]),
            "json_pointer",
            DomainError::NotABodyPointer,
        ),
        (
            field("", is, &["main"]),
            "json_pointer",
            DomainError::NotABodyPointer,
        ),
        (field("/ref", is, &[]), "values", DomainError::Empty),
        (field("/ref", is, &[" "]), "values[0]", DomainError::Empty),
        (
            field("/ref", is, &["a", "b"]),
            "values",
            DomainError::TooMany { max: 1 },
        ),
        (
            field("/ref", one_of, &["a", "a"]),
            "values[1]",
            DomainError::Duplicate,
        ),
        (
            field("/ref", ConditionOperator::Matches, &["release/["]),
            "values[0]",
            DomainError::NotAGlob,
        ),
    ];
    for (condition, path, error) in cases {
        assert_eq!(
            refusal(condition),
            (format!("webhook.conditions[0].{path}"), error)
        );
    }
}

/// A webhook has at most 32 conditions.
#[test]
fn the_check_refuses_too_many_conditions() {
    // given
    let condition = header("X-GitHub-Event", ConditionOperator::Is, &["push"]);
    let rule = WebhookRule {
        conditions: vec![condition; MAX_CONDITIONS + 1],
    };

    // when
    let refused = rule.check("webhook");

    // then
    let error = DomainError::TooMany {
        max: MAX_CONDITIONS,
    };
    assert_eq!(refused, Err(("webhook.conditions".to_owned(), error)));
}
