use crate::domain::text::{Rule, Text};

pub enum OrganizationNameRule {}

impl Rule for OrganizationNameRule {
    const LABEL: &'static str = "Organization name";
    const MAX: usize = 255;
}

pub type OrganizationName = Text<OrganizationNameRule>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_and_trimming() {
        assert!(OrganizationName::new("Valid Org").is_ok());
        let trimmed = OrganizationName::new("  My Org  ").unwrap();
        assert_eq!(trimmed.as_str(), "My Org");
        assert!(OrganizationName::new("").is_err());
        assert!(OrganizationName::new("   ").is_err());
    }

    #[test]
    fn enforces_length_bounds() {
        let max = OrganizationName::new("a".repeat(OrganizationNameRule::MAX)).unwrap();
        assert_eq!(max.as_str().len(), OrganizationNameRule::MAX);
        assert!(OrganizationName::new("a".repeat(OrganizationNameRule::MAX + 1)).is_err());
    }
}
