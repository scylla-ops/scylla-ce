use crate::application::{PasswordResetDelivery, PasswordResetMessage, PasswordResetSender};
use crate::domain::errors::DomainResult;
use async_trait::async_trait;
use tracing::info;

/// The sender of an edition without mail: one INFO line with the link, which an operator gives
/// to the user. The line never holds the email.
#[derive(Debug, Default, Clone, Copy)]
pub struct LogPasswordResetSender;

#[async_trait]
impl PasswordResetSender for LogPasswordResetSender {
    fn delivery(&self) -> PasswordResetDelivery {
        PasswordResetDelivery::ServerLog
    }

    async fn send(&self, message: &PasswordResetMessage) -> DomainResult<()> {
        info!(
            target: "scylla_core",
            user_id = %message.user_id,
            username = %message.username,
            link = %message.link,
            expires_at = %message.expires_at,
            "password reset link",
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::clock;
    use crate::domain::ids::UserId;
    use crate::domain::user::{DisplayName, Email, Username};
    use std::fmt::{Debug, Write};
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Level, Metadata, Subscriber};

    #[derive(Default, Clone)]
    struct Lines(Arc<Mutex<Vec<(Level, String, String)>>>);

    struct Text(String);

    impl Visit for Text {
        fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
            let _ = write!(self.0, "{}={:?} ", field.name(), value);
        }
    }

    impl Subscriber for Lines {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }
        fn record(&self, _: &Id, _: &Record<'_>) {}
        fn record_follows_from(&self, _: &Id, _: &Id) {}
        fn event(&self, event: &Event<'_>) {
            let mut text = Text(String::new());
            event.record(&mut text);
            let meta = event.metadata();
            self.0
                .lock()
                .unwrap()
                .push((*meta.level(), meta.target().to_owned(), text.0));
        }
        fn enter(&self, _: &Id) {}
        fn exit(&self, _: &Id) {}
    }

    #[test]
    fn one_info_line_holds_the_link_and_never_the_email() {
        let lines = Lines::default();
        let message = PasswordResetMessage {
            user_id: UserId::new("u1"),
            email: Email::new("kevin@example.com").unwrap(),
            username: Username::new("kevin").unwrap(),
            display_name: Some(DisplayName::new("Kevin").unwrap()),
            link: "http://127.0.0.1:8080/reset-password#token=abc".into(),
            expires_at: clock::now(),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();

        tracing::subscriber::with_default(lines.clone(), || {
            runtime
                .block_on(LogPasswordResetSender.send(&message))
                .unwrap();
        });

        let lines = lines.0.lock().unwrap();
        assert_eq!(lines.len(), 1);
        let (level, target, text) = &lines[0];
        assert_eq!(*level, Level::INFO);
        assert_eq!(target, "scylla_core");
        assert!(text.contains("u1"), "{text}");
        assert!(text.contains("kevin"), "{text}");
        assert!(text.contains(&message.link), "{text}");
        assert!(!text.contains("example.com"), "{text}");
        assert_eq!(
            LogPasswordResetSender.delivery(),
            PasswordResetDelivery::ServerLog
        );
    }
}
