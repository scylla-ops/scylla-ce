use scylla_parity::Edition;
use std::process::ExitCode;

fn main() -> ExitCode {
    scylla_parity::main(&Edition::ce())
}
