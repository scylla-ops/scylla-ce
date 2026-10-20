use crate::domain::text::{Description, Rule, Text};

pub enum RoleNameRule {}

impl Rule for RoleNameRule {
    const LABEL: &'static str = "Role name";
    const MAX: usize = 255;
}

/// The grant's role key; the label people read is a [`RoleDisplayName`].
pub type RoleName = Text<RoleNameRule>;

pub enum RoleDisplayNameRule {}

impl Rule for RoleDisplayNameRule {
    const LABEL: &'static str = "Role name";
    const MAX: usize = 255;
}

pub type RoleDisplayName = Text<RoleDisplayNameRule>;

pub type RoleDescription = Description;
