//! What the server tells about itself, the capabilities of the edition, the one-off import of the
//! old product, the settings of the instance, its legal settings, its mail settings, the records
//! of the consent, the uploaded images, and the retention of the personal data.

pub(crate) mod capabilities;
pub(crate) mod consent;
pub(crate) mod images;
pub mod import;
pub(crate) mod legal;
pub(crate) mod mail_settings;
pub mod retention;
pub(crate) mod settings;
