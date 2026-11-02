---
name: add-dependency
description: Add or change a third-party crate in the Scylla workspace, with its features in [workspace.dependencies] and a check of the compile time. Use before you add a crate to a Cargo.toml, or change the version or the features of a dependency.
---

# Add a dependency

Compile time is a requirement of this project. Each dependency adds time to each cold build, and sometimes to each rebuild.

## Before you add it

1. Count the crates in the lock file, and keep the number: `grep -c '^name = ' Cargo.lock`.
2. If the user names the crate, use it. If not, find out if `std` or a crate that the workspace already has can do the work. Look at `[workspace.dependencies]` in the root `Cargo.toml`.
3. Find out which crate needs it. The crate decides the cost:

| Crate | Permitted dependencies |
|---|---|
| `scylla-domain` | Small pure crates only, like `uuid`, `time`, `thiserror`. Never `sqlx`, `tonic`, `tokio`, `prost` or `async-trait`. |
| `scylla-extension` | The domain crate, `sqlx`, `async-trait`, `thiserror`, `tonic` (only for `RoutesBuilder`). |
| `scylla-proto` | `prost`, `prost-types`, `tonic`, `tonic-prost`. |
| `scylla-core` | What the features need. |
| `scylla-testkit` | The core, the domain crate, the proto crate, `sqlx`, `tokio`, `tonic`. |
| `scylla-ce` | The core, the extension crate, `tokio`. |

`scripts/check-structure.sh` stops a heavy crate in a domain crate.

## Steps

1. Find the current version: `cargo search <name> --limit 1`.

2. Add the crate to `[workspace.dependencies]` in the root `Cargo.toml`, in the group of third-party crates (after the empty line below the `scylla-*` lines), in alphabetical order.
   - Write the version with no operator, for example `"1.0.140"`.
   - Add `default-features = false` if the default features turn on code that the workspace does not use.
   - Write only the features that the code uses.

3. In the `Cargo.toml` of the crate, add only `<name>.workspace = true`, in alphabetical order, in `[dependencies]` or `[dev-dependencies]`. Never write `features` or a version there.
   - Then all crates use the same feature set, and `cargo build` and `cargo test` share their artifacts.
   - `scripts/check-structure.sh` stops a version or features in the `Cargo.toml` of a crate.
   - Do not use `cargo add`. It writes the version and the features into the `Cargo.toml` of the crate.

4. Run `cargo build --workspace`. Then run `cargo build -p scylla-ce`. The last command must compile only the bin crate, not tokio or sqlx again. If it compiles a dependency again, the features are not the same. Move the features to `[workspace.dependencies]`.

5. Look for a second version of a crate with `cargo tree --workspace -d`. If the new crate adds one, try a version of the new crate that uses the same version as the workspace.

6. The Enterprise repository keeps its third-party lines equal to the lines of this repository. If you change the version or the features of a crate that it also uses (`sqlx`, `tokio`, `tonic`, `prost`, `thiserror`), tell the user in the report.

## Measure

Count the crates in the lock file again, and compare with the number of step 1 of "Before you add it".

Run `just measure` in these cases:

- The new crate has a proc macro.
- The new crate adds more than 10 crates to `Cargo.lock`.
- The new crate goes into a crate that many crates depend on: `scylla-domain`, `scylla-extension` or `scylla-proto`.

The measure takes about 3 minutes. Write the new values in the table "Compile times" of `README.md`. One run is not precise: the cold build changed by 5 s between two runs with almost no code change. Report a cost only if it is larger than this.

## Finish

Load the `finish-change` skill and do all of it.
