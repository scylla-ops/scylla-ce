---
name: finish-change
description: The checklist that ends each change in the Scylla public repository, which means format, SQL cache, docs, lint, structure checks, tests, and the report to the user. Use it before you tell the user that a change is done. Other skills load it as their last step.
---

# Finish a change

Do the steps in order. If a step fails, repair the cause and start again.

1. `cargo fmt --all`.
2. A schema change is a new migration after the last one, with its line at the end of `migrations.lock` (decision 39). Never change, remove or rename a file of the base branch or of a release tag, and never ask for a reset of a database. The change keeps the data and the server of the last version working: add first, and remove an old column or table only in a later version. A rename or a change of type of a column is expand, migrate, then contract in a later version (`AGENT.md`, SQL). If you changed your own new file after your dev database applied it: change its lock line, and run `just db-reset` on your dev database. If SQL or a migration changed: `just prepare`, and keep `.sqlx/` in the change.
3. Docs: `README.md` if the crates, commands or tests changed. `DECISIONS.md` if a decision is new or changed. `AGENT.md` and the skills if a rule changed. ASD-STE100, no em dashes.
4. `just lint`: fmt, clippy with the doc lints, `scripts/check-structure.sh`. If a proto changed: `just lint` and `buf breaking` (its `AGENT.md`) in `scylla-protos`, and the new pin of the submodule `proto` in the change. Do not change the script to pass: tell the user which rule stops the change.
5. Read the diff: call order, one transaction per service call, no commit in a hook, short comments, no reason in the code.
6. `just test`. All tests pass.
7. Report: what changed, the test result, what you did not do, each new decision, each change to the API for the editions.
8. Never commit without the permission of the user. Message: `<type>: <description>`, no body, no footer.
