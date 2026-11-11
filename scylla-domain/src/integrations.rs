//! The Git integration: the GitHub App of the instance, the connections of a workspace to GitHub
//! or GitLab, the repository of a project, the commit of a job, its checkout, its reports, and the
//! merge rules.

pub mod apps;
pub mod checkout;
pub mod connections;
pub mod links;
pub mod merge_blocks;
pub mod reports;
pub mod sources;
#[cfg(test)]
mod tests;

/// Declares an enum whose values have a name in the database, with `as_str`, `parse` and `ALL`.
macro_rules! named {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($(#[$value_meta:meta])* $value:ident = $text:literal,)+
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$value_meta])* $value,)+
        }

        impl $name {
            /// Each value, in the order of the declaration.
            pub const ALL: &'static [Self] = &[$(Self::$value,)+];

            /// Returns the name of the value, as the database keeps it.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$value => $text,)+
                }
            }

            /// Reads the name of a value.
            pub fn parse(name: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|value| value.as_str() == name)
            }
        }
    };
}

pub(crate) use named;

named! {
    /// A Git provider.
    pub enum VcsProvider {
        /// GitHub.com, through a GitHub App.
        GitHub = "github",
        /// GitLab.com, through the token of a service account.
        GitLab = "gitlab",
    }
}
