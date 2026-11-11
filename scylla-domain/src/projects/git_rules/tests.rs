//! Unit tests of the rules of the Git triggers.

use super::{
    GitRules, PullRequestAction, PullRequestRule, PushRule, TagRule, TriggerEvent, MAX_GLOBS,
};
use crate::DomainError;

/// Makes a list of globs.
fn globs(globs: &[&str]) -> Vec<String> {
    globs.iter().map(|glob| (*glob).to_owned()).collect()
}

/// Makes a push event on a branch.
fn push<'a>(branch: &'a str, message: &'a str) -> TriggerEvent<'a> {
    TriggerEvent::Push { branch, message }
}

/// A push rule takes its branches, where `*` stops at a `/`, and not the excluded ones.
#[test]
fn a_push_rule_takes_its_branches() {
    // given
    let rules = GitRules {
        push: Some(PushRule {
            branches: globs(&["main", "release/*"]),
            exclude: globs(&["release/old"]),
            ..PushRule::default()
        }),
        ..GitRules::default()
    };

    // when
    let branches = [
        "main",
        "release/1.0",
        "release/1.0/fix",
        "release/old",
        "feature/a",
    ]
    .map(|branch| rules.matches(&push(branch, "Fix the build")));

    // then
    assert_eq!(branches, [true, true, false, false, false]);
}

/// A push with `[skip ci]` or `[ci skip]` starts no run, unless `skip_ci` is off.
#[test]
fn a_push_with_a_skip_mark_starts_no_run() {
    // given
    let skip = PushRule::default();
    let keep = PushRule {
        skip_ci: false,
        ..PushRule::default()
    };

    // when
    let messages = ["Fix [skip ci]", "Docs\n\n[CI SKIP]", "Fix the build"].map(|message| {
        (
            skip.matches(&push("main", message)),
            keep.matches(&push("main", message)),
        )
    });

    // then
    assert_eq!(messages, [(false, true), (false, true), (true, true)]);
}

/// A pull request rule takes its target branches and actions, and skips a draft unless
/// `drafts` is on.
#[test]
fn a_pull_request_rule_skips_the_drafts() {
    // given
    let rule = PullRequestRule {
        branches: globs(&["main"]),
        events: vec![PullRequestAction::Opened, PullRequestAction::ReadyForReview],
        drafts: false,
    };
    let with_drafts = PullRequestRule {
        drafts: true,
        ..rule.clone()
    };
    let event = |target, action, draft| TriggerEvent::PullRequest {
        target,
        action,
        draft,
    };

    // when
    let results = [
        rule.matches(&event("main", PullRequestAction::Opened, false)),
        rule.matches(&event("main", PullRequestAction::Opened, true)),
        rule.matches(&event("main", PullRequestAction::ReadyForReview, false)),
        rule.matches(&event("main", PullRequestAction::NewCommits, false)),
        rule.matches(&event("dev", PullRequestAction::Opened, false)),
        with_drafts.matches(&event("main", PullRequestAction::Opened, true)),
    ];

    // then
    assert_eq!(results, [true, false, true, false, false, true]);
}

/// A tag rule takes its patterns, and an event of another kind never passes a rule.
#[test]
fn a_tag_rule_takes_its_patterns() {
    // given
    let rules = GitRules {
        tag: Some(TagRule {
            patterns: globs(&["v*"]),
        }),
        ..GitRules::default()
    };

    // when
    let results = [
        rules.matches(&TriggerEvent::Tag { name: "v1.2.0" }),
        rules.matches(&TriggerEvent::Tag { name: "nightly" }),
        rules.matches(&push("v1", "")),
    ];

    // then
    assert_eq!(results, [true, false, false]);
}

/// A glob that does not compile, an empty glob and too many globs are refused at their path.
#[test]
fn bad_globs_are_refused() {
    // given
    let rules = GitRules {
        push: Some(PushRule {
            branches: globs(&["main", "[main", ""]),
            exclude: vec!["x".to_owned(); MAX_GLOBS + 1],
            skip_ci: true,
        }),
        ..GitRules::default()
    };

    // when
    let refusals = rules.check("on");

    // then
    let expected = [
        ("on.push.branches[1]", DomainError::NotAGlob),
        ("on.push.branches[2]", DomainError::NotAGlob),
        ("on.push.exclude", DomainError::TooMany { max: MAX_GLOBS }),
    ];
    let refusals: Vec<_> = refusals
        .iter()
        .map(|(path, error)| (path.as_str(), error.clone()))
        .collect();
    assert_eq!(refusals, expected);
}

/// The JSON form of the rules, which the triggers keep, reads back to the same rules, with the
/// defaults not written.
#[test]
fn the_rules_read_back_from_json() {
    // given
    let rules = GitRules {
        push: Some(PushRule::default()),
        pull_request: Some(PullRequestRule {
            events: vec![PullRequestAction::NewCommits],
            ..PullRequestRule::default()
        }),
        tag: None,
    };

    // when
    let json = serde_json::to_value(&rules).unwrap();
    let read_back: GitRules = serde_json::from_value(json.clone()).unwrap();

    // then
    let expected = serde_json::json!({ "push": {}, "pull_request": { "events": ["new_commits"] } });
    assert_eq!(json, expected);
    assert_eq!(read_back, rules);
}
