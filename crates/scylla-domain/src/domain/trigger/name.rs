use crate::domain::text::{Rule, Text};

pub enum TriggerNameRule {}

impl Rule for TriggerNameRule {
    const LABEL: &'static str = "Trigger name";
    const MAX: usize = 255;
}

pub type TriggerName = Text<TriggerNameRule>;
