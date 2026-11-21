//! The tests of the platform: capabilities, health, reflection, gRPC-Web, the HTTP/JSON gateway,
//! the legal settings, TLS, the route rule, the logs, the files of the web app and the pool of the
//! test server.

mod capabilities;
mod grpc_web;
mod health;
mod import;
mod json;
mod legal;
mod logs;
mod reflection;
mod routes;
mod test_server;
mod tls;
mod web;
