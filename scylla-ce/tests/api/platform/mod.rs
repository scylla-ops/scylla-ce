//! The tests of the platform: capabilities, health, reflection, gRPC-Web, the HTTP/JSON gateway,
//! the legal settings, the consent, TLS, the route rule, the logs, the client IP address of the
//! logs, the retention, the tasks of an edition, the files of the web app and the pool of the
//! test server.

mod capabilities;
mod client_ip;
mod consent;
mod grpc_web;
mod health;
mod import;
mod json;
mod legal;
mod logs;
mod reflection;
mod retention;
mod routes;
mod tasks;
mod test_server;
mod tls;
mod web;
