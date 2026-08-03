# `example-plugin` Design

## Responsibility

`example-plugin` is an executable integration example for the compile-time plugin contracts. It
demonstrates plugin-owned payloads, sockets, graph-node registrations, runtime processing,
renderer registration, and an optional application panel.

## Boundaries

The plugin is linked explicitly by enabled application composition and submits registrations through
inventory. Its graph/runtime portion depends on graph API and generic runtime contracts. Its camera
demonstration also uses the supported UI and viewer extension contracts, which is the optional
presentation boundary identified for future split in
[Crate Responsibility Design](../architecture/crate_responsibility.md).
