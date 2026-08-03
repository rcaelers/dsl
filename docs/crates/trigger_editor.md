# `trigger_editor` Design

## Responsibility

`trigger_editor` owns the reusable widget that edits generic trigger-program contracts.

## Boundaries

It depends on `signal_processing` trigger values and egui. It does not discover a capture source,
validate device-specific trigger capabilities, persist graph state, or start acquisition. The UI
and concrete source feature supply the applicable contract and own resulting edits.
