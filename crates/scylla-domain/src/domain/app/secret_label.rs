use crate::domain::text::{Rule, Text};

pub enum AppSecretLabelRule {}

impl Rule for AppSecretLabelRule {
    const LABEL: &'static str = "App secret label";
    const MAX: usize = 64;
}

pub type AppSecretLabel = Text<AppSecretLabelRule>;

pub const DEFAULT_SECRET_LABEL: &str = "default";

/// The label of the first secret when the caller gives none: `name` without its control
/// characters, cut to [`AppSecretLabelRule::MAX`] bytes, else [`DEFAULT_SECRET_LABEL`].
#[must_use]
pub fn default_secret_label(name: &str) -> AppSecretLabel {
    let kept: String = name.chars().filter(|c| !c.is_control()).collect();
    let kept = kept.trim();
    let end = kept
        .char_indices()
        .map(|(start, c)| start + c.len_utf8())
        .take_while(|&end| end <= AppSecretLabelRule::MAX)
        .last()
        .unwrap_or(0);
    AppSecretLabel::new(&kept[..end])
        .or_else(|_| AppSecretLabel::new(DEFAULT_SECRET_LABEL))
        .expect("the default label is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_and_trimming() {
        assert!(AppSecretLabel::new("ci-runner").is_ok());
        assert_eq!(
            AppSecretLabel::new("  default  ").unwrap().as_str(),
            "default"
        );
        assert!(AppSecretLabel::new("").is_err());
        assert!(AppSecretLabel::new("   ").is_err());
    }

    #[test]
    fn enforces_length_bound() {
        assert!(AppSecretLabel::new("a".repeat(AppSecretLabelRule::MAX)).is_ok());
        assert!(AppSecretLabel::new("a".repeat(AppSecretLabelRule::MAX + 1)).is_err());
    }

    #[test]
    fn a_valid_name_is_the_default_label_as_it_is() {
        assert_eq!(
            default_secret_label("Build-01.example.com").as_str(),
            "Build-01.example.com"
        );
    }

    #[test]
    fn a_name_loses_its_control_characters_and_its_bytes_after_the_limit() {
        assert_eq!(default_secret_label(" ci\tbot\u{7f} ").as_str(), "cibot");

        let long = format!("{}é", "a".repeat(AppSecretLabelRule::MAX - 1));
        assert_eq!(
            default_secret_label(&long).as_str(),
            "a".repeat(AppSecretLabelRule::MAX - 1)
        );

        let spaced = format!("{} b", "a".repeat(AppSecretLabelRule::MAX - 1));
        assert_eq!(
            default_secret_label(&spaced).as_str(),
            "a".repeat(AppSecretLabelRule::MAX - 1)
        );
    }

    #[test]
    fn a_name_with_no_valid_character_gives_the_fallback() {
        assert_eq!(
            default_secret_label("\u{1}\t \n").as_str(),
            DEFAULT_SECRET_LABEL
        );
        assert_eq!(default_secret_label("").as_str(), DEFAULT_SECRET_LABEL);
    }
}
