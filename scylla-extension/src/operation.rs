//! The operation of a request: its gRPC method, the permission of its route, and what the gate
//! saw during the request.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The operation of a request. The server adds it to each request, and the gate writes on it.
/// A token can restrict the operations, and each route checks its own permission.
#[derive(Clone, Debug)]
pub struct Operation {
    /// The gRPC method, for example `scylla.projects.v1.ProjectService/CreateProject`.
    method: String,
    /// The permission of the route: the method name in snake case, for example `create_project`.
    permission: String,
    /// What the gate saw, shared by the copies of the operation.
    state: Arc<State>,
}

/// What the gate saw during a request.
#[derive(Debug, Default)]
struct State {
    /// The gate verified the token of the request.
    authenticated: AtomicBool,
    /// A check of the route permission passed.
    authorized: AtomicBool,
}

impl Operation {
    /// Reads the operation from the path of a request: `/<service>/<method>`.
    pub fn from_path(path: &str) -> Self {
        let method = path.trim_start_matches('/');
        let name = method.rsplit_once('/').map_or(method, |(_, name)| name);
        Self {
            method: method.to_owned(),
            permission: snake_case(name),
            state: Arc::default(),
        }
    }

    /// Returns the gRPC method.
    pub fn as_str(&self) -> &str {
        &self.method
    }

    /// Returns the permission of the route.
    pub fn permission(&self) -> &str {
        &self.permission
    }

    /// Records that the gate verified the token of the request.
    pub fn set_authenticated(&self) {
        self.state.authenticated.store(true, Ordering::SeqCst);
    }

    /// Records that a check of the route permission passed.
    pub fn set_authorized(&self) {
        self.state.authorized.store(true, Ordering::SeqCst);
    }

    /// Returns `true` if the gate verified the token, but no check of the route permission
    /// passed.
    pub fn is_unchecked(&self) -> bool {
        self.state.authenticated.load(Ordering::SeqCst)
            && !self.state.authorized.load(Ordering::SeqCst)
    }
}

/// Returns a name in snake case: `RegisterAgent` gives `register_agent`, and `GetHTTPServer`
/// gives `get_http_server`.
fn snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut text = String::with_capacity(name.len() + 4);
    for (index, &c) in chars.iter().enumerate() {
        // Keep each character that is not an upper-case letter.
        if !c.is_ascii_uppercase() {
            text.push(c);
            continue;
        }

        // Start a new word after a word, or at the last letter of an acronym.
        let previous = index.checked_sub(1).map(|before| chars[before]);
        let next = chars.get(index + 1);
        let after_word = previous.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit());
        let after_acronym = previous.is_some_and(|p| p.is_ascii_uppercase())
            && next.is_some_and(|n| n.is_ascii_lowercase());
        if after_word || after_acronym {
            text.push('_');
        }
        text.push(c.to_ascii_lowercase());
    }
    text
}

/// Unit tests of the operation: the names, and the records of the gate.
#[cfg(test)]
mod tests {
    use super::Operation;

    /// The permission of a route is the method name in snake case, with acronyms and digits.
    #[test]
    fn the_route_permission_is_the_method_in_snake_case() {
        // given
        let paths = [
            "/scylla.projects.v1.ProjectService/CreateProject",
            "/scylla.agents.v1.AgentService/RegisterAgent",
            "/scylla.auth.v1.TokenService/ExchangeIdToken",
            "/scylla.test.v1.TestService/GetHTTPServer2Name",
        ];

        // when
        let operations = paths.map(Operation::from_path);

        // then
        let permissions = operations.each_ref().map(Operation::permission);
        let expected = [
            "create_project",
            "register_agent",
            "exchange_id_token",
            "get_http_server2_name",
        ];
        assert_eq!(permissions, expected);
        assert_eq!(
            operations[0].as_str(),
            "scylla.projects.v1.ProjectService/CreateProject"
        );
    }

    /// A copy of an operation sees what the gate records on the other copy.
    #[test]
    fn a_copy_sees_the_records_of_the_gate() {
        // given
        let operation = Operation::from_path("/scylla.projects.v1.ProjectService/ListProjects");
        let copy = operation.clone();

        // when
        copy.set_authenticated();
        let after_token = operation.is_unchecked();
        copy.set_authorized();
        let after_check = operation.is_unchecked();

        // then
        assert!(after_token);
        assert!(!after_check);
    }
}
