//! The kernel of the domain: the ids and their format, the names, the pages, the kinds of
//! resources and the errors. The crate root gives each type.

mod codec;
mod error;
mod id;
mod name;
mod page;
mod resource;

pub use error::DomainError;
pub use id::Id;
pub use name::Name;
pub use page::{Page, PageRequest, PageSize};
pub use resource::ResourceKind;
