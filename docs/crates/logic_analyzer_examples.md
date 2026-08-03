# `logic-analyzer-examples` Design

## Responsibility

The workspace-root `logic-analyzer-examples` package owns standalone examples, benchmarks, and
cross-crate integration tests. It demonstrates composition without becoming a reusable application
or core-domain owner.

## Boundaries

Its dependencies are development-only composition dependencies. Reusable crates do not depend on
it, and feature behavior is not implemented here. Test-only graph fixtures remain in the workspace
integration-test support package; editable graph examples remain user-facing documents rather than
code dependencies.
