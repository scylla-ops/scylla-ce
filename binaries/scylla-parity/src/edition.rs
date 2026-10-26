use crate::server_log::LogLine;

pub struct Edition {
    pub name: &'static str,
    pub descriptor_sets: Vec<&'static [u8]>,
    pub scenarios: Vec<&'static str>,
    pub contracts: Vec<&'static str>,
    pub config_extra: &'static str,
    pub server_env: Vec<(&'static str, &'static str)>,
    /// The slices that the B server of an acceptance must serve on strata.
    pub strata_slices: Vec<&'static str>,
    /// The workers that the B server of an acceptance must run on strata.
    pub strata_workers: Vec<&'static str>,
    /// The log targets that the servers show at info, in addition to scylla_strata and scylla_core.
    pub log_targets: Vec<&'static str>,
    /// The lines that the log of the B server must show exactly once, in addition to the CE lines.
    pub required_lines: Vec<LogLine>,
}

impl Edition {
    #[must_use]
    pub fn ce() -> Self {
        Self {
            name: "ce",
            descriptor_sets: vec![scylla_proto::FILE_DESCRIPTOR_SET],
            scenarios: vec![include_str!("../data/scenario.ce.json")],
            contracts: vec![include_str!("../data/contract.ce.json")],
            config_extra: "",
            server_env: Vec::new(),
            strata_slices: vec![
                "projects",
                "secrets",
                "pipelines",
                "apps",
                "roles",
                "grants",
                "organizations",
                "users",
                "auth",
                "triggers",
                "jobs",
                "agents",
            ],
            strata_workers: vec!["CronTriggers", "DispatchJobs", "ReapJobs", "SweepSessions"],
            log_targets: Vec::new(),
            required_lines: Vec::new(),
        }
    }

    #[must_use]
    pub fn extend(
        mut self,
        name: &'static str,
        descriptor_set: &'static [u8],
        scenario: &'static str,
        contract: &'static str,
    ) -> Self {
        self.name = name;
        self.descriptor_sets.push(descriptor_set);
        self.scenarios.push(scenario);
        self.contracts.push(contract);
        self
    }

    /// The `RUST_LOG` of the servers: `warn`, then `info` for scylla_strata, scylla_core and each log target.
    #[must_use]
    pub fn rust_log(&self) -> String {
        ["scylla_strata", "scylla_core"]
            .iter()
            .chain(&self.log_targets)
            .fold(String::from("warn"), |filter, target| {
                format!("{filter},{target}=info")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_targets_of_an_edition_go_to_info() {
        let mut edition = Edition::ce();
        assert_eq!(
            edition.rust_log(),
            "warn,scylla_strata=info,scylla_core=info"
        );
        edition.log_targets.push("scylla_ee");
        assert_eq!(
            edition.rust_log(),
            "warn,scylla_strata=info,scylla_core=info,scylla_ee=info"
        );
    }
}
