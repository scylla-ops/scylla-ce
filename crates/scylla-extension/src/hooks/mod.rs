//! The extension seam: six positions around each stage, three erased and three typed.

mod extension;
mod next;
mod position;
mod registry;

pub use extension::Extension;
pub use next::{Done, Next, Proceed};
pub use position::{Around, Gate, Listener, Observer, Policy, Wrap};
pub use registry::Hooks;
