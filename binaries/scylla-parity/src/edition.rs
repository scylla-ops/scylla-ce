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
}
