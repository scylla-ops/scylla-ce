---
name: scope
description: Interview the user to turn a vague request into a precise spec. Manual only.
disable-model-invocation: true
---
Request: $ARGUMENTS

Interview me with the AskUserQuestion tool until the task is fully scoped. Do not implement.
Read the related code before you ask, so each question is specific to this codebase.
Ask one round at a time, up to 4 questions per round. Do not ask obvious questions. Dig into the hard parts.
Cover: goal, users, files and modules touched, data and edge cases, constraints, verification, out of scope.
Stop when no question is open. Then write `SPEC.md`: goal, files, acceptance criteria, edge cases, out of scope, verification step.
