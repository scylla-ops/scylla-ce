//! The generated code of `proto/`, and the version text of a build of the server or of the agent.
//! Docs come from the proto comments, which the lint of `scylla-protos` checks.

/// The activity API: the feed of the changes of a workspace.
pub mod activity {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.activity.v1");
    }
}

/// The capabilities API.
pub mod capabilities {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.capabilities.v1");
    }
}

/// The shared messages: the id and the page.
pub mod common {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.common.v1");
    }
}

/// The dashboards API: the saved layouts of the dashboard of a workspace.
pub mod dashboards {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.dashboards.v1");
    }
}

/// The integrations API: the connections to GitHub and GitLab, the repository of a project, the
/// merge rules and the reports of the jobs.
pub mod integrations {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.integrations.v1");
    }
}

/// The notifications API: the inbox of a user.
pub mod notifications {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.notifications.v1");
    }
}

/// The projects API: the projects and their secrets.
pub mod projects {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.projects.v1");
    }
}

/// The pipelines API: the pipelines of the projects, their jobs and the logs of the jobs.
pub mod pipelines {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.pipelines.v1");
    }
}

/// The Apps API: the machine identities of a workspace.
pub mod apps {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.apps.v1");
    }
}

/// The agents API: the stream of an App that runs jobs.
pub mod agents {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.agents.v1");
    }
}

/// The token API.
pub mod auth {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.auth.v1");
    }
}

/// The access API: the grants of roles.
pub mod access {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.access.v1");
    }
}

/// The workspaces API.
pub mod workspaces {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.workspaces.v1");
    }
}

/// The accounts API: sign-up, sign-in with a password, the profile and the personal tokens of
/// the caller, and the console of the users.
pub mod accounts {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.accounts.v1");
    }
}

/// The platform API: the settings and the GitHub App of the instance.
pub mod platform {
    /// Version 1.
    #[allow(missing_docs, clippy::missing_docs_in_private_items)]
    pub mod v1 {
        tonic::include_proto!("scylla.platform.v1");
    }
}

/// The descriptors of each proto file, for the JSON gateway.
pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/scylla_descriptors.bin"));

/// The version text that a build of the server or of the agent reports: in `--version`, in the
/// hello of the agent and in `GetCapabilities`. The downloads use the bare `CARGO_PKG_VERSION`.
pub mod version {
    use std::sync::LazyLock;

    /// The commit of this build, from `SCYLLA_BUILD` at the build, or empty.
    pub const BUILD: &str = match option_env!("SCYLLA_BUILD") {
        Some(build) => build,
        None => "",
    };

    /// The version text of this build. Only the release workflow sets `SCYLLA_RELEASE`.
    static TEXT: LazyLock<String> = LazyLock::new(|| {
        let release = option_env!("SCYLLA_RELEASE") == Some("true");
        of(env!("CARGO_PKG_VERSION"), release, BUILD)
    });

    /// Returns the version text of this build.
    pub fn text() -> &'static str {
        &TEXT
    }

    /// Returns `version` for a release build. Else adds the first 7 characters of the commit as
    /// build metadata (`0.7.0+240da00`), or `dev` with no commit (`0.7.0+dev`).
    fn of(version: &str, release: bool, build: &str) -> String {
        if release {
            return version.to_owned();
        }
        let commit = build.get(..7).unwrap_or(build);
        let metadata = if commit.is_empty() { "dev" } else { commit };
        format!("{version}+{metadata}")
    }

    #[cfg(test)]
    mod tests {
        //! Unit tests of the version text of a build.

        use super::of;

        /// Only a release build reports the bare version. Another build adds its short commit,
        /// or `dev` with no commit.
        #[test]
        fn the_version_text_tells_the_build() {
            // given
            let builds = [
                (
                    true,
                    "240da00c4dfdf313619d59f1ed389a65b4357189",
                    "0.7.0-rc.1",
                ),
                (true, "", "0.7.0-rc.1"),
                (
                    false,
                    "240da00c4dfdf313619d59f1ed389a65b4357189",
                    "0.7.0-rc.1+240da00",
                ),
                (false, "1a2b", "0.7.0-rc.1+1a2b"),
                (false, "", "0.7.0-rc.1+dev"),
            ];

            // when
            let texts: Vec<String> = builds
                .iter()
                .map(|(release, build, _)| of("0.7.0-rc.1", *release, build))
                .collect();

            // then
            let expected: Vec<&str> = builds.iter().map(|(_, _, text)| *text).collect();
            assert_eq!(texts, expected);
        }
    }
}
