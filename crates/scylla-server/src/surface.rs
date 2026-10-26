#[derive(Default)]
pub struct Surface {
    pub(crate) http: Vec<axum::Router>,
    pub(crate) descriptors: Vec<&'static [u8]>,
}

impl Surface {
    pub fn file_descriptor_set(&mut self, set: &'static [u8]) -> &mut Self {
        self.descriptors.push(set);
        self
    }

    /// The router must not define a fallback: the web UI owns it.
    pub fn http_routes(&mut self, router: axum::Router) -> &mut Self {
        self.http.push(router);
        self
    }
}
