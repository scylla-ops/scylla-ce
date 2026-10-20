use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::text::{Rule, Text};
use serde::{Deserialize, Serialize};

pub const MAX_ARGS: usize = 256;

pub enum ExecCommandRule {}

impl Rule for ExecCommandRule {
    const LABEL: &'static str = "Exec command";
    const MAX: usize = 4096;
}

pub type ExecCommand = Text<ExecCommandRule>;

pub enum ExecArgRule {}

impl Rule for ExecArgRule {
    const LABEL: &'static str = "Exec argument";
    const MAX: usize = 65_536;
    const REQUIRED: bool = false;
    const FREE_FORM: bool = true;

    fn sanitize(raw: String) -> String {
        raw
    }
}

pub type ExecArg = Text<ExecArgRule>;

pub enum ScriptBodyRule {}

impl Rule for ScriptBodyRule {
    const LABEL: &'static str = "Script";
    const MAX: usize = 1_048_576;
    const FREE_FORM: bool = true;

    fn sanitize(raw: String) -> String {
        raw
    }
}

pub type ScriptBody = Text<ScriptBodyRule>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shell {
    #[default]
    Sh,
    Bash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    Exec {
        command: ExecCommand,
        args: Vec<ExecArg>,
    },
    Script {
        script: ScriptBody,
        shell: Shell,
    },
}

impl Step {
    pub fn exec(command: String, args: Vec<String>) -> DomainResult<Self> {
        if args.len() > MAX_ARGS {
            return Err(DomainError::validation(format!(
                "Exec step cannot have more than {MAX_ARGS} arguments"
            )));
        }
        Ok(Self::Exec {
            command: ExecCommand::new(command)?,
            args: args
                .into_iter()
                .map(ExecArg::new)
                .collect::<DomainResult<_>>()?,
        })
    }

    pub fn script(script: String, shell: Shell) -> DomainResult<Self> {
        Ok(Self::Script {
            script: ScriptBody::new(script)?,
            shell,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_rejects_blank_command() {
        assert!(Step::exec("   ".into(), vec![]).is_err());
        assert!(Step::exec("echo".into(), vec!["hi".into()]).is_ok());
    }

    #[test]
    fn script_rejects_blank_script() {
        assert!(Step::script("  \n ".into(), Shell::Sh).is_err());
        assert!(Step::script("echo hi".into(), Shell::Bash).is_ok());
    }

    #[test]
    fn exec_bounds_its_arguments() {
        let args = |n: usize| vec![String::from("a"); n];
        assert!(Step::exec("echo".into(), args(MAX_ARGS)).is_ok());
        assert!(Step::exec("echo".into(), args(MAX_ARGS + 1)).is_err());
        assert!(Step::exec("echo".into(), vec!["a".repeat(ExecArgRule::MAX + 1)]).is_err());
        assert!(Step::exec("echo".into(), vec![String::new(), " x ".into()]).is_ok());
    }

    #[test]
    fn nul_is_rejected_everywhere() {
        assert!(Step::exec("ec\0ho".into(), vec![]).is_err());
        assert!(Step::exec("echo".into(), vec!["a\0".into()]).is_err());
        assert!(Step::script("echo \0".into(), Shell::Sh).is_err());
    }

    #[test]
    fn script_keeps_its_text_and_is_bounded() {
        let Step::Script { script, .. } = Step::script("\techo hi\n".into(), Shell::Sh).unwrap()
        else {
            unreachable!()
        };
        assert_eq!(script.as_str(), "\techo hi\n");
        assert!(Step::script("a".repeat(ScriptBodyRule::MAX + 1), Shell::Sh).is_err());
    }

    #[test]
    fn step_json_is_tagged() {
        let exec = Step::exec("ls".into(), vec!["-la".into()]).unwrap();
        let json = serde_json::to_string(&exec).unwrap();
        assert!(json.contains(r#""kind":"exec""#), "{json}");

        let script = Step::script("make".into(), Shell::Sh).unwrap();
        let json = serde_json::to_string(&script).unwrap();
        assert!(json.contains(r#""kind":"script""#), "{json}");
        assert!(json.contains(r#""shell":"sh""#), "{json}");
    }
}
