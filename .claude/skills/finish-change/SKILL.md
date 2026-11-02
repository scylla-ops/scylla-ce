---
name: finish-change
description: The checklist that ends each change in the Scylla repository, which means format, SQL cache, docs, lint, structure checks, tests, and the report to the user. Use it before you tell the user that a change is done. Other skills load it as their last step.
---

# Finish a change

Do all the steps, in this order. If a step fails, repair the cause and start again at step 1. Do not tell the user that the change is done before step 6 passes.

1. **Format.** Run `cargo fmt --all`.

2. **SQL cache.** If you changed a migration or a `query!` call (also in a test), run `just prepare`.

3. **Docs.** If the change adds a workspace crate, a feature, an RPC, a file, a rule or a command, update these files:
   - `README.md`: the tree, the rules, the recipe "Add a feature", the tables.
   - `AGENT.md`: the rules and the list of skills.
   - The skill in `.claude/skills/` that tells the procedure. A skill that is not correct is worse than no skill. If you moved code, change each skill that names the old place.

   Write all docs and skills in ASD-STE100 Simplified Technical English. Do not use em dashes.

4. **Lint.** Run `just lint`. It runs these commands:
   - `cargo fmt --all --check`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `buf lint`
   - `scripts/check-structure.sh`

   `scripts/check-structure.sh` checks these rules:
   - No EE word (`quotas?`, `plans?`, `tenants?`, `enterprise`, as whole words) in a `.rs`, `.proto` or `.sql` file of a crate or of `proto/`.
   - No `cfg(feature` in a `scylla-*/src` folder.
   - No `.rs` file in a `scylla-*` crate has more than 300 lines.
   - Each `mod.rs` and `lib.rs`, except in `scylla-proto`, has only `mod`, `use` and `#[cfg(test)]` lines.
   - No `model.rs` in a feature folder.
   - A `grpc.rs` never calls a store.
   - A `store.rs` never names tonic or a proto crate.
   - `scylla-core` does not re-export a domain type.
   - No heavy dependency in a domain crate.
   - No version and no features in the `Cargo.toml` of a crate.
   - No line of a Rust file starts with a `//` comment. `///` and `//!` are permitted.
   - No em dash in a file of the repository.

   If the script stops a change that you think is correct, do not change the script. Tell the user which rule stops the change, and why you think that the change is correct. Do not add the name of an EE feature to the word list here. The Enterprise repository checks its own feature names in the crates of this repository.
5. **Rules that the script cannot check.** Read each file that you changed. With Git, read the diff. Make sure of these points:
   - The call order is `grpc -> service -> guard -> store`.
   - Each write is in one transaction that the service opens and commits.
   - A guard never commits. It writes only to its own tables.
   - Each domain type comes from its domain crate. Each call names its module (`store::insert`, not `insert`).
   - A `//` comment after code on the same line is not permitted. The script does not find it.
   - Each `///` doc comment gives a reason that the code cannot show, like a correctness argument, and nothing else.

6. **Tests.** Run `just test`. All tests must pass. Keep the `test result` lines for the report.

7. **Report.** Tell the user these items:
   - What changed, for each file or each feature.
   - The test result: the number of tests, and pass or fail.
   - What you did not do, and why.
   - Each decision that no skill gave, and each rule that you changed, with its reason.
   - Each change to the API for the editions (`README.md`, section "The API for the editions"). The Enterprise build can stop at the next tag.

8. **Commit.** Never commit without explicit permission from the user. When the user agrees, write the message in the form `<type>: <description>`, with no body and no footer. `<type>` is one of `feat`, `fix`, `refactor`, `test`, `docs`, `build`, `chore`.
