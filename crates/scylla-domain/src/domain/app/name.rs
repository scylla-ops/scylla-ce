use crate::domain::text::{Rule, Text};

pub enum AppNameRule {}

impl Rule for AppNameRule {
    const LABEL: &'static str = "App name";
    const MAX: usize = 255;
}

pub type AppName = Text<AppNameRule>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_and_trimming() {
        assert!(AppName::new("ci-runner").is_ok());
        assert_eq!(AppName::new("  bot  ").unwrap().as_str(), "bot");
        assert!(AppName::new("").is_err());
        assert!(AppName::new("   ").is_err());
    }

    #[test]
    fn enforces_length_bound() {
        assert!(AppName::new("a".repeat(AppNameRule::MAX)).is_ok());
        assert!(AppName::new("a".repeat(AppNameRule::MAX + 1)).is_err());
    }
}
