//! The images that a person uploads: the square PNG that the server makes from an upload, the
//! table that keeps it, and the route that gives it to the browsers.

mod http;
mod raster;
pub(crate) mod service;
pub(crate) mod store;

#[cfg(test)]
mod tests;

pub(crate) use http::router;
