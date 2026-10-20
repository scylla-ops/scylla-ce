use scylla_auth::authz::Visibility;

/// A `Visibility` as query parameters: `all`, or the rows of these organizations and projects.
#[derive(Default)]
pub struct VisibilityFilter {
    pub(crate) all: bool,
    pub(crate) orgs: Vec<String>,
    pub(crate) projects: Vec<String>,
}

impl VisibilityFilter {
    #[must_use]
    pub fn new(visible: &Visibility) -> Self {
        match visible {
            Visibility::All => Self {
                all: true,
                ..Self::default()
            },
            Visibility::Scoped { orgs, projects } => Self {
                all: false,
                orgs: orgs.iter().map(|o| o.as_str().to_owned()).collect(),
                projects: projects.iter().map(|p| p.as_str().to_owned()).collect(),
            },
        }
    }
}
