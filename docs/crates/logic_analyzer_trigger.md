# `logic_analyzer_trigger` Design

## Responsibility

`logic_analyzer_trigger` owns the portable logic-analyzer trigger domain: opaque simple digital
conditions, serializable trigger programs and predicates, provider capability schemas, stable
identifiers, edit classification, and validation diagnostics.

The crate depends only on `signal_capture::CaptureChannelId` plus portable serialization and error
libraries. It has no acquisition-session, driver, graph, widget, UI, device, protocol, or platform
dependency. Editors, viewers, compiler features, concrete acquisition nodes, and application
composition import its crate-root facade directly.

## Contract boundary

Schemas describe supported trigger grammar without naming a device or interpreting registered
predicate identifiers. Programs persist stable schema and operand identities. Validation checks
format, schema revision, stage limits, channel membership, predicate operands, and provider
capabilities. Concrete acquisition owners remain responsible for lowering a validated program to
hardware or host-side execution.
