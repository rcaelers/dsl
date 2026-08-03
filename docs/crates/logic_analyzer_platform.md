# `logic_analyzer_platform` Design

## Responsibility

`logic_analyzer_platform` implements host capabilities owned by core crates and selects native or
web adapters at complete module boundaries. It returns an opaque composition bundle to application
roots.

## Facade and dependencies

The crate root exposes `PlatformServices` and standard service constructors. Private platform
modules implement dialogs, repositories, file access, worker transport, native Sigrok hosting,
USB-related factories, and unavailable web capabilities through contracts from their behavioral
owners.

## Ownership boundaries

It does not define alternative graph, capture, runtime, or UI data models and is never a dependency
of a reusable core crate. Platform-specific dependencies and target selection are confined here as
defined by [Unified Native and Web Storage Platform Design](../aspects/native_web_storage.md).
