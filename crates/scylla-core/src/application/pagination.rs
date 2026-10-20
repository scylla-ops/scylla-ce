use crate::domain::errors::{DomainError, DomainResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaginationParams {
    page: u32,
    page_size: u32,
}

impl PaginationParams {
    pub const DEFAULT_PAGE_SIZE: u32 = 20;
    pub const MAX_PAGE_SIZE: u32 = 100;
    pub const MIN_PAGE: u32 = 1;

    pub fn new(page: u32, page_size: u32) -> DomainResult<Self> {
        if page < Self::MIN_PAGE {
            return Err(DomainError::validation(format!(
                "Page must be at least {}",
                Self::MIN_PAGE
            )));
        }

        if page_size == 0 {
            return Err(DomainError::validation(
                "Page size must be greater than 0".to_string(),
            ));
        }

        if page_size > Self::MAX_PAGE_SIZE {
            return Err(DomainError::validation(format!(
                "Page size cannot exceed {}",
                Self::MAX_PAGE_SIZE
            )));
        }

        Ok(Self { page, page_size })
    }

    #[must_use]
    pub fn page(&self) -> u32 {
        self.page
    }

    #[must_use]
    pub fn page_size(&self) -> u32 {
        self.page_size
    }

    #[must_use]
    pub fn offset(&self) -> u64 {
        u64::from(self.page - 1) * u64::from(self.page_size)
    }

    #[must_use]
    pub fn limit(&self) -> u64 {
        u64::from(self.page_size)
    }
}

impl Default for PaginationParams {
    fn default() -> Self {
        Self {
            page: Self::MIN_PAGE,
            page_size: Self::DEFAULT_PAGE_SIZE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaginationMetadata {
    total_count: u64,
    page: u32,
    page_size: u32,
    total_pages: u32,
    has_next: bool,
    has_previous: bool,
}

impl PaginationMetadata {
    #[must_use]
    pub fn new(params: &PaginationParams, total_count: u64) -> Self {
        let total_pages = if total_count == 0 {
            0
        } else {
            u32::try_from(total_count.div_ceil(u64::from(params.page_size))).unwrap_or(u32::MAX)
        };

        let has_next = params.page < total_pages;
        let has_previous = params.page > PaginationParams::MIN_PAGE;

        Self {
            total_count,
            page: params.page,
            page_size: params.page_size,
            total_pages,
            has_next,
            has_previous,
        }
    }

    #[must_use]
    pub fn total_count(&self) -> u64 {
        self.total_count
    }

    #[must_use]
    pub fn page(&self) -> u32 {
        self.page
    }

    #[must_use]
    pub fn page_size(&self) -> u32 {
        self.page_size
    }

    #[must_use]
    pub fn total_pages(&self) -> u32 {
        self.total_pages
    }

    #[must_use]
    pub fn has_next(&self) -> bool {
        self.has_next
    }

    #[must_use]
    pub fn has_previous(&self) -> bool {
        self.has_previous
    }
}

#[derive(Debug, Clone)]
pub struct PaginatedResult<T> {
    items: Vec<T>,
    metadata: PaginationMetadata,
}

impl<T> PaginatedResult<T> {
    #[must_use]
    pub fn new(items: Vec<T>, params: &PaginationParams, total_count: u64) -> Self {
        let metadata = PaginationMetadata::new(params, total_count);
        Self { items, metadata }
    }

    #[must_use]
    pub fn items(&self) -> &Vec<T> {
        &self.items
    }

    #[must_use]
    pub fn metadata(&self) -> &PaginationMetadata {
        &self.metadata
    }

    #[must_use]
    pub fn into_parts(self) -> (Vec<T>, PaginationMetadata) {
        (self.items, self.metadata)
    }

    /// A page whose items were resolved from another page's ids keeps that page's metadata.
    #[must_use]
    pub fn from_parts(items: Vec<T>, metadata: PaginationMetadata) -> Self {
        Self { items, metadata }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pagination_params_valid() {
        let params = PaginationParams::new(1, 20).unwrap();
        assert_eq!(params.page(), 1);
        assert_eq!(params.page_size(), 20);
        assert_eq!(params.offset(), 0);
        assert_eq!(params.limit(), 20);
    }

    #[test]
    fn test_pagination_params_offset_calculation() {
        let params = PaginationParams::new(3, 20).unwrap();
        assert_eq!(params.offset(), 40);
    }

    #[test]
    fn test_pagination_params_invalid_page() {
        let result = PaginationParams::new(0, 20);
        assert!(result.is_err());
    }

    #[test]
    fn test_pagination_params_invalid_page_size_zero() {
        let result = PaginationParams::new(1, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_pagination_params_invalid_page_size_too_large() {
        let result = PaginationParams::new(1, 101);
        assert!(result.is_err());
    }

    #[test]
    fn test_pagination_metadata() {
        let params = PaginationParams::new(2, 20).unwrap();
        let metadata = PaginationMetadata::new(&params, 100);

        assert_eq!(metadata.total_count(), 100);
        assert_eq!(metadata.page(), 2);
        assert_eq!(metadata.page_size(), 20);
        assert_eq!(metadata.total_pages(), 5);
        assert!(metadata.has_next());
        assert!(metadata.has_previous());
    }

    #[test]
    fn test_pagination_metadata_first_page() {
        let params = PaginationParams::new(1, 20).unwrap();
        let metadata = PaginationMetadata::new(&params, 100);

        assert!(metadata.has_next());
        assert!(!metadata.has_previous());
    }

    #[test]
    fn test_pagination_metadata_last_page() {
        let params = PaginationParams::new(5, 20).unwrap();
        let metadata = PaginationMetadata::new(&params, 100);

        assert!(!metadata.has_next());
        assert!(metadata.has_previous());
    }

    #[test]
    fn test_pagination_metadata_empty_results() {
        let params = PaginationParams::new(1, 20).unwrap();
        let metadata = PaginationMetadata::new(&params, 0);

        assert_eq!(metadata.total_pages(), 0);
        assert!(!metadata.has_next());
        assert!(!metadata.has_previous());
    }

    #[test]
    fn test_paginated_result() {
        let params = PaginationParams::new(1, 20).unwrap();
        let items = vec![1, 2, 3];
        let result = PaginatedResult::new(items.clone(), &params, 100);

        assert_eq!(result.items(), &items);
        assert_eq!(result.metadata().total_count(), 100);
    }
}
