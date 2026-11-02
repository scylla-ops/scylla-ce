#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceKind(&'static str);

impl ResourceKind {
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}
