//! The images that a person uploads: the square PNG that the server makes from an upload, the
//! SVG of a glyph avatar that the server draws, the table that keeps them, and the route that
//! gives them to the browsers.

mod glyph;
mod http;
mod raster;
pub(crate) mod service;
pub(crate) mod store;

#[cfg(test)]
mod tests;

pub(crate) use http::router;
