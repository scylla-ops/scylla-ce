use crate::domain::text::{Rule, Text};

pub enum ProjectNameRule {}

impl Rule for ProjectNameRule {
    const LABEL: &'static str = "Project name";
    const MAX: usize = 255;
}

pub type ProjectName = Text<ProjectNameRule>;
