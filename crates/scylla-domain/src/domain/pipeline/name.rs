use crate::domain::text::{Rule, Text};

pub enum PipelineNameRule {}

impl Rule for PipelineNameRule {
    const LABEL: &'static str = "Pipeline name";
    const MAX: usize = 255;
}

pub type PipelineName = Text<PipelineNameRule>;
