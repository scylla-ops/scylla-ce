use scylla_domain::domain::errors::DomainError;
use tonic::Status;
use tracing::error;

/// Not a `From` impl: both types are foreign (orphan rule).
pub fn domain_error_to_status(err: DomainError) -> Status {
    match err {
        DomainError::NotFound(message) => Status::not_found(message),
        DomainError::Validation(message) => Status::invalid_argument(message),
        DomainError::BusinessRule(message) => Status::failed_precondition(message),
        DomainError::Unauthorized(message) => Status::unauthenticated(message),
        DomainError::Forbidden(message) => Status::permission_denied(message),
        DomainError::Conflict(message) => Status::already_exists(message),
        DomainError::Stale(message) => Status::aborted(message),
        DomainError::QuotaExceeded(message) => Status::resource_exhausted(message),
        DomainError::Infrastructure(message) => {
            error!("Infrastructure error: {}", message);
            Status::internal("Internal server error")
        }
        DomainError::Internal(message) => {
            error!("Internal error: {}", message);
            Status::internal("Internal server error")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn not_found_maps_to_not_found() {
        let err = DomainError::not_found("User", "123");
        let status = domain_error_to_status(err);
        assert_eq!(status.code(), Code::NotFound);
        assert_eq!(status.message(), "User '123' not found");
    }

    #[test]
    fn stale_maps_to_aborted() {
        let status = domain_error_to_status(DomainError::stale("Project", "p1"));
        assert_eq!(status.code(), Code::Aborted);
        assert!(
            status
                .message()
                .starts_with("Project 'p1' changed since it was read")
        );
    }

    #[test]
    fn validation_maps_to_invalid_argument() {
        let err = DomainError::validation("bad input");
        let status = domain_error_to_status(err);
        assert_eq!(status.code(), Code::InvalidArgument);
    }

    #[test]
    fn business_rule_maps_to_failed_precondition() {
        let err = DomainError::business_rule("rule violation");
        let status = domain_error_to_status(err);
        assert_eq!(status.code(), Code::FailedPrecondition);
    }

    #[test]
    fn unauthorized_maps_to_unauthenticated() {
        let err = DomainError::unauthorized("not logged in");
        let status = domain_error_to_status(err);
        assert_eq!(status.code(), Code::Unauthenticated);
    }

    #[test]
    fn forbidden_maps_to_permission_denied() {
        let err = DomainError::forbidden("no access");
        let status = domain_error_to_status(err);
        assert_eq!(status.code(), Code::PermissionDenied);
    }

    #[test]
    fn conflict_maps_to_already_exists() {
        let err = DomainError::conflict("duplicate");
        let status = domain_error_to_status(err);
        assert_eq!(status.code(), Code::AlreadyExists);
    }

    #[test]
    fn quota_exceeded_maps_to_resource_exhausted() {
        let err = DomainError::quota_exceeded("project quota reached");
        let status = domain_error_to_status(err);
        assert_eq!(status.code(), Code::ResourceExhausted);
    }

    #[test]
    fn infrastructure_maps_to_internal() {
        let err = DomainError::infrastructure("db error");
        let status = domain_error_to_status(err);
        assert_eq!(status.code(), Code::Internal);
        assert_eq!(status.message(), "Internal server error");
    }

    #[test]
    fn internal_maps_to_internal() {
        let err = DomainError::internal("panic");
        let status = domain_error_to_status(err);
        assert_eq!(status.code(), Code::Internal);
        assert_eq!(status.message(), "Internal server error");
    }
}
