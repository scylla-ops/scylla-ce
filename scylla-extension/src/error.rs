#[derive(Debug, thiserror::Error)]
pub enum GuardError {
    #[error("{}", .0.message)]
    Rejected(Rejection),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug)]
pub struct Rejection {
    pub code: &'static str,
    pub message: String,
    pub details: Vec<(&'static str, i64)>,
}

impl Rejection {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: Vec::new(),
        }
    }

    pub fn detail(mut self, key: &'static str, value: i64) -> Self {
        self.details.push((key, value));
        self
    }
}

impl From<Rejection> for GuardError {
    fn from(rejection: Rejection) -> Self {
        Self::Rejected(rejection)
    }
}
