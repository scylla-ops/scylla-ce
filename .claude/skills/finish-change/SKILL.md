---
name: finish-change
description: The checklist that ends each change in the Scylla public repository, which means format, SQL cache, docs, lint, structure checks, tests, and the report to the user. Use it before you tell the user that a change is done. Other skills load it as their last step.
---

# Finish a change

Do the steps in order. If a step fails, repair the cause and start again.

1. `cargo fmt --all`.
2. If a migration changed: `just db-reset`. If SQL or a migration changed: `just prepare`, and keep `.sqlx/` in the change.
3. Docs: `README.md` if the crates, commands or tests changed. `DECISIONS.md` if a decision is new or changed. `AGENT.md` and the skills if a rule changed. ASD-STE100, no em dashes.
4. `just lint`: fmt, clippy with the doc lints, `scripts/check-structure.sh`. If a proto changed: `just lint` in `scylla-protos`, and the new pin of the submodule `proto` in the change. Do not change the script to pass: tell the user which rule stops the change.
5. Read the diff: call order, one transaction per service call, no commit in a hook, short comments, no reason in the code.
6. `just test`. All tests pass.
7. Report: what changed, the test result, what you did not do, each new decision, each change to the API for the editions.
8. Never commit without the permission of the user. Message: `<type>: <description>`, no body, no footer.
