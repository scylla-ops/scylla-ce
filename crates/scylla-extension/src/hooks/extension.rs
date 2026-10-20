use super::registry::Hooks;
use std::sync::Arc;

/// The receiver is the `Arc`, so one instance registers itself at several positions with
/// `self.clone()` and the binary keeps the `Arc` to read its state.
pub trait Extension {
    fn register(self: &Arc<Self>, hooks: &mut Hooks);
}
