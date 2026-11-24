//! The slugs of the workspaces: short unique names for the URLs.

use crate::DomainError;

/// The names that are not slugs: the top-level routes of the web apps (the CE app and the cloud
/// app) and of the server, and names that they can use later. A workspace URL is
/// `<app>/<slug>`, so a slug must not hide a route.
const RESERVED: [&str; 28] = [
    "accept-invitation",
    "account",
    "admin",
    "api",
    "assets",
    "auth",
    "change-email",
    "extensions",
    "feedback",
    "forgot-password",
    "health",
    "invite",
    "login",
    "logout",
    "members",
    "metrics",
    "new",
    "projects",
    "register",
    "reset-password",
    "settings",
    "sign-in",
    "sign-up",
    "static",
    "users",
    "verify-email",
    "webhooks",
    "workspaces",
];

/// The slug of a workspace: a short unique name for the URLs. It has 2 to 39 lower case
/// letters, digits or dashes, and starts and ends with a letter or a digit.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Slug(String);

impl Slug {
    /// The minimum number of characters.
    pub const MIN_LEN: usize = 2;
    /// The maximum number of characters.
    pub const MAX_LEN: usize = 39;
    /// The slug of a name that gives fewer than `MIN_LEN` letters and digits.
    const FALLBACK: &'static str = "workspace";

    /// Trims the text, makes it lower case, and checks its form. A reserved name is refused.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        let slug = Self(raw.trim().to_lowercase());
        if !is_valid(&slug.0) {
            return Err(DomainError::NotASlug);
        }
        if slug.is_reserved() {
            return Err(DomainError::ReservedSlug);
        }
        Ok(slug)
    }

    /// Returns `true` if the slug is a reserved name: a derived slug then takes a suffix.
    pub fn is_reserved(&self) -> bool {
        RESERVED.contains(&self.0.as_str())
    }

    /// Makes a slug from a name: ASCII letters in lower case, digits, and one dash for each run
    /// of other characters, cut to `MAX_LEN`. A result shorter than `MIN_LEN` gives `workspace`.
    /// The result can be a reserved name.
    pub fn derive(name: &str) -> Self {
        // Keep the letters and the digits, with one dash between two runs.
        let ascii = deunicode::deunicode(name).to_ascii_lowercase();
        let mut slug = String::with_capacity(ascii.len());
        for c in ascii.chars() {
            if c.is_ascii_alphanumeric() {
                slug.push(c);
            } else if !slug.is_empty() && !slug.ends_with('-') {
                slug.push('-');
            }
        }

        // Cut it to the maximum length, with no dash at the end.
        let slug = cut(&slug, Self::MAX_LEN);
        if slug.len() < Self::MIN_LEN {
            return Self(Self::FALLBACK.to_owned());
        }
        Self(slug.to_owned())
    }

    /// Returns the slug with the suffix `-<number>`, cut to fit `MAX_LEN`: `team` and 2 give
    /// `team-2`.
    pub fn with_suffix(&self, number: u32) -> Self {
        let suffix = format!("-{number}");
        let base = cut(&self.0, Self::MAX_LEN - suffix.len());
        Self(format!("{base}{suffix}"))
    }

    /// Returns the slug as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Returns `true` if a text has the form of a slug.
fn is_valid(slug: &str) -> bool {
    let edge = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    (Slug::MIN_LEN..=Slug::MAX_LEN).contains(&slug.len())
        && slug.chars().all(|c| edge(c) || c == '-')
        && slug.starts_with(edge)
        && slug.ends_with(edge)
}

/// Returns the first `max` characters of an ASCII slug, with no dash at the end.
fn cut(slug: &str, max: usize) -> &str {
    slug[..slug.len().min(max)].trim_end_matches('-')
}

#[cfg(test)]
mod tests {
    //! Unit tests of the slugs: the form, the derivation from a name, and the suffix.

    use super::Slug;
    use crate::DomainError;

    /// A slug has 2 to 39 characters, starts and ends with a letter or a digit, and has only
    /// lower case letters, digits and dashes. The server trims it and makes it lower case.
    #[test]
    fn parse_checks_the_form() {
        // given
        let (longest, too_long) = ("x".repeat(39), "x".repeat(40));
        let valid = ["ab", "a-b", "a--b", " Team-42 ", longest.as_str()];
        let invalid = [
            "a",
            "",
            "-ab",
            "ab-",
            "a_b",
            "a.b",
            "a b",
            "é-team",
            too_long.as_str(),
        ];

        // when
        let parsed: Vec<_> = valid.map(|raw| Slug::parse(raw.to_owned())).into();
        let refused: Vec<_> = invalid.map(|raw| Slug::parse(raw.to_owned())).into();

        // then
        assert_eq!(parsed[3].as_ref().map(Slug::as_str), Ok("team-42"));
        assert!(parsed.iter().all(Result::is_ok), "{parsed:?}");
        assert!(refused.iter().all(|r| r == &Err(DomainError::NotASlug)));
    }

    /// A name gives its letters in ASCII and lower case, with one dash between two runs.
    #[test]
    fn derive_makes_a_slug_of_a_name() {
        // given
        let names = ["  Équipe Été! ", "john.doe_x", "Team 42", "日本"];

        // when
        let slugs = names.map(|name| Slug::derive(name).as_str().to_owned());

        // then
        assert_eq!(slugs, ["equipe-ete", "john-doe-x", "team-42", "ri-ben"]);
    }

    /// A name with fewer than two letters and digits gives `workspace`, and a long name is cut
    /// to 39 characters with no dash at the end.
    #[test]
    fn derive_keeps_the_limits() {
        // given
        let long = format!("{} {}", "a".repeat(37), "bcd");

        // when
        let short = [Slug::derive("!!!"), Slug::derive("a"), Slug::derive("")];
        let cut = Slug::derive(&long);

        // then
        assert!(short.iter().all(|slug| slug.as_str() == "workspace"));
        assert_eq!(cut.as_str(), format!("{}-b", "a".repeat(37)));
        assert!(Slug::parse(cut.as_str().to_owned()).is_ok());
    }

    /// A reserved name is not a slug, in any case. A name can give a reserved slug, and its
    /// suffix is not reserved.
    #[test]
    fn a_reserved_name_is_not_a_slug() {
        // given
        let names = ["login", " Settings ", "auth", "webhooks"];

        // when
        let parsed = names.map(|name| Slug::parse(name.to_owned()));
        let derived = Slug::derive("Login");

        // then
        assert!(parsed.iter().all(|r| r == &Err(DomainError::ReservedSlug)));
        assert!(derived.is_reserved() && !derived.with_suffix(2).is_reserved());
    }

    /// A suffix goes after the slug, and a long slug is cut so the result has 39 characters.
    #[test]
    fn with_suffix_fits_the_maximum() {
        // given
        let short = Slug::derive("team");
        let long = Slug::derive(&format!("{}-{}", "a".repeat(30), "b".repeat(8)));

        // when
        let (second, tenth) = (short.with_suffix(2), long.with_suffix(10));

        // then
        assert_eq!(second.as_str(), "team-2");
        assert_eq!(
            tenth.as_str(),
            format!("{}-{}-10", "a".repeat(30), "b".repeat(5))
        );
        assert!(Slug::parse(tenth.as_str().to_owned()).is_ok());
    }
}
