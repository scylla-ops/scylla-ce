/// `Parse` for a request whose fields are each `id(wire_field)`, `optional_id(wire_field)`,
/// `page` or `copy`.
macro_rules! parse {
    (@field $self:ident, $field:ident, id($wire:ident)) => {
        $crate::grpc::convert::id($self.$wire, stringify!($wire))?
    };
    (@field $self:ident, $field:ident, optional_id($wire:ident)) => {
        $crate::grpc::convert::optional_id($self.$wire, stringify!($wire))?
    };
    (@field $self:ident, $field:ident, page) => {
        $crate::grpc::mappers::proto_to_domain_pagination($self.pagination)
    };
    (@field $self:ident, $field:ident, copy) => {
        $self.$field
    };
    ($request:ty => $action:ident) => {
        impl $crate::grpc::convert::Parse for $request {
            type Into = $action;

            fn parse(self) -> Result<$action, ::tonic::Status> {
                Ok($action)
            }
        }
    };
    ($request:ty => $action:ident { $($field:ident: $kind:ident $(($wire:ident))?),+ $(,)? }) => {
        impl $crate::grpc::convert::Parse for $request {
            type Into = $action;

            fn parse(self) -> Result<$action, ::tonic::Status> {
                Ok($action {
                    $($field: parse!(@field self, $field, $kind $(($wire))?),)+
                })
            }
        }
    };
}

/// `From<PaginatedResult<T>>` for list responses that carry `items.map(to_proto)` and the page.
macro_rules! page_response {
    ($item:ty => $items:ident: $to_proto:path; $($response:ty),+ $(,)?) => {$(
        impl From<$crate::application::pagination::PaginatedResult<$item>> for $response {
            fn from(page: $crate::application::pagination::PaginatedResult<$item>) -> Self {
                let (items, metadata) = page.into_parts();
                Self {
                    $items: items.iter().map($to_proto).collect(),
                    pagination: Some($crate::grpc::mappers::domain_to_proto_metadata(&metadata)),
                }
            }
        }
    )+};
}

/// A `PrincipalAccess` as one of its wire rows: `UserAccess` and `AppAccess` have the same fields.
macro_rules! access_to_proto {
    ($proto:ident, $access:expr) => {{
        let access: &$crate::application::PrincipalAccess = $access;
        $proto {
            grant_id: $crate::grpc::convert::wrap(access.grant_id.clone()),
            scope: Some($crate::grpc::convert::scope_ref_to_proto(&access.scope)),
            organization_id: access
                .organization_id
                .as_ref()
                .and_then(|id| $crate::grpc::convert::wrap(id.to_string())),
            organization_name: access
                .organization_name
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            project_name: access
                .project_name
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            role_id: $crate::grpc::convert::wrap(access.role.to_string()),
            role_name: access.role_name.to_string(),
        }
    }};
}
