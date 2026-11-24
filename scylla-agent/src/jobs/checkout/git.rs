//! The git of the machine, as the hello finds it, and the settings that each git command of a
//! checkout gets.

use std::process::Stdio;

use scylla_proto::agents::v1::DispatchCheckout;
use tokio::process::Command;

/// The feature that the hello tells when git runs on the machine.
const FEATURE: &str = "checkout";

/// The variable of the environment of git that holds the header for `--config-env`.
const HEADER_VAR: &str = "SCYLLA_CHECKOUT_HEADER";

/// The git of the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Git {
    /// `true` when git takes `--config-env`: version 2.31 or newer.
    config_env: bool,
}

/// The settings of each git command of one checkout.
pub(super) struct Config {
    /// The options before the command.
    args: Vec<String>,
    /// The header, in the variable `SCYLLA_CHECKOUT_HEADER` of git.
    header: Option<String>,
    /// `true` when the header is in the options, because git is older than 2.31.
    header_in_args: bool,
}

impl Git {
    /// Finds the git of the machine: `None` when `git --version` does not run.
    pub(crate) async fn detect() -> Option<Self> {
        let output = Command::new("git")
            .arg("--version")
            .stdin(Stdio::null())
            .output()
            .await
            .ok()?;
        output
            .status
            .success()
            .then(|| Self::parse(&String::from_utf8_lossy(&output.stdout)))
    }

    /// Reads the output of `git --version`. A version that it cannot read is an old one.
    pub(super) fn parse(output: &str) -> Self {
        let text = output
            .trim()
            .strip_prefix("git version ")
            .unwrap_or_default();
        let mut numbers = text
            .split(|character: char| !character.is_ascii_digit())
            .map(|number| number.parse::<u32>().ok());
        let version = (numbers.next().flatten(), numbers.next().flatten());
        Self {
            config_env: matches!(version, (Some(major), Some(minor)) if (major, minor) >= (2, 31)),
        }
    }
}

/// Returns the features of the agent: `checkout` when git runs.
pub(crate) fn features(git: Option<Git>) -> Vec<String> {
    git.map(|_| FEATURE.to_owned()).into_iter().collect()
}

impl Config {
    /// Makes the settings of a checkout: no credential helper, and the header only for the clone
    /// URL. With git 2.31 or newer, the header is in a variable and not in the arguments.
    pub(super) fn of(checkout: &DispatchCheckout, git: Option<Git>) -> Self {
        let mut args: Vec<String> = [
            "credential.helper=",
            "init.defaultBranch=main",
            "advice.detachedHead=false",
        ]
        .into_iter()
        .flat_map(|setting| ["-c".to_owned(), setting.to_owned()])
        .collect();
        let header = &checkout.auth_header;
        if header.is_empty() {
            return Self {
                args,
                header: None,
                header_in_args: false,
            };
        }
        let name = format!("http.{}.extraHeader", checkout.clone_url);
        if git.is_some_and(|git| git.config_env) {
            args.push(format!("--config-env={name}={HEADER_VAR}"));
            return Self {
                args,
                header: Some(header.clone()),
                header_in_args: false,
            };
        }
        args.extend(["-c".to_owned(), format!("{name}={header}")]);
        Self {
            args,
            header: None,
            header_in_args: true,
        }
    }

    /// Returns `true` when the header is in the options, because git is older than 2.31.
    pub(super) fn header_in_args(&self) -> bool {
        self.header_in_args
    }

    /// Adds the options and the variable of the header to a git command.
    pub(super) fn apply(&self, command: &mut Command) {
        command.args(&self.args);
        if let Some(header) = &self.header {
            command.env(HEADER_VAR, header);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests of the version of git and of the arguments of its commands.

    use scylla_proto::agents::v1::DispatchCheckout;
    use tokio::process::Command;

    use super::{Config, Git, HEADER_VAR};

    /// A checkout with a header.
    fn checkout() -> DispatchCheckout {
        DispatchCheckout {
            clone_url: "https://github.com/acme/api.git".to_owned(),
            auth_header: "Authorization: Basic s3cr3t-token".to_owned(),
            ..DispatchCheckout::default()
        }
    }

    /// Returns the arguments of a git command with the settings of a checkout.
    fn args(config: &Config) -> (Vec<String>, Command) {
        let mut command = Command::new("git");
        config.apply(&mut command);
        let args = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        (args, command)
    }

    /// The version comes from the first two numbers, and 2.31 is the first with `--config-env`.
    #[test]
    fn the_version_of_git_is_read() {
        // given
        let outputs = [
            ("git version 2.39.3 (Apple Git-146)\n", true),
            ("git version 2.56.0.windows.4", true),
            ("git version 2.31.0", true),
            ("git version 3.0.0", true),
            ("git version 2.30.2", false),
            ("git version 1.99.9", false),
            ("not git", false),
        ];

        // when
        let read: Vec<bool> = outputs
            .iter()
            .map(|(output, _)| Git::parse(output).config_env)
            .collect();

        // then
        let expected: Vec<bool> = outputs.iter().map(|(_, config_env)| *config_env).collect();
        assert_eq!(read, expected);
    }

    /// With git 2.31 or newer, the header is in a variable of git and never in its arguments.
    #[test]
    fn a_new_git_gets_the_header_in_a_variable() {
        // given
        let git = Git::parse("git version 2.31.0");

        // when
        let config = Config::of(&checkout(), Some(git));
        let (args, command) = args(&config);

        // then
        assert!(args.iter().all(|arg| !arg.contains("s3cr3t")), "{args:?}");
        let option =
            format!("--config-env=http.https://github.com/acme/api.git.extraHeader={HEADER_VAR}");
        assert!(args.contains(&option), "{args:?}");
        let header = command
            .as_std()
            .get_envs()
            .find(|(name, _)| *name == HEADER_VAR)
            .and_then(|(_, value)| value);
        assert_eq!(header.unwrap(), "Authorization: Basic s3cr3t-token");
        assert!(!config.header_in_args());
    }

    /// An older git gets the header with `-c`, and the agent can tell it.
    #[test]
    fn an_old_git_gets_the_header_in_its_arguments() {
        // given
        let git = Git::parse("git version 2.30.2");

        // when
        let config = Config::of(&checkout(), Some(git));
        let (args, _) = args(&config);

        // then
        let setting =
            "http.https://github.com/acme/api.git.extraHeader=Authorization: Basic s3cr3t-token";
        assert_eq!(
            args[args.len() - 2..],
            ["-c".to_owned(), setting.to_owned()]
        );
        assert!(config.header_in_args());
    }
}
