# `logic_analyzer_processing` Design

## Responsibility

`logic_analyzer_processing` owns concrete, UI-independent logic-analyser behavior: capture
formats and devices, protocol decoders, processing nodes, and sinks. It owns format and transport
errors until it crosses a generic `signal_processing` contract.

## Facade and dependencies

The root exports shared source metadata and process-node construction. Its public `nodes` and
`types` namespaces expose supported concrete-node contracts. It depends on `signal_processing`
and uses `logic_analyzer_test_support` only for deterministic test fixtures.

## Ownership boundaries

Graph definitions, saved-node migration, socket styling, renderer registration, UI controls, and
platform selection are outside this crate. Host adapters inject file, device, and execution
capabilities through its contracts. The temporary native adapter leaves are constrained by
[Responsibility and Visibility Design](../aspects/responsibility_visibility.md).
