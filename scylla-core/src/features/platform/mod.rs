//! What the server tells about itself, the capabilities of the edition, the one-off import of the
//! old product, the settings of the instance, its legal settings, the records of the consent, and
//! the retention of the personal data.

pub(crate) mod capabilities;
pub(crate) mod consent;
pub mod import;
pub(crate) mod legal;
pub mod retention;
pub(crate) mod settings;
