# `logic_analyzer_graph_api` Design

## Responsibility

`logic_analyzer_graph_api` owns the compile-time extension contracts used by graph-node and
payload-plugin features. It defines inventory registrations, runtime-builder contracts, payload
registration, port identities, resolved inputs, and UI-independent presentation descriptions.

## Facade and dependencies

The public `node` namespace contains submitted or implemented feature contracts. The public
`node_support` namespace contains their values and restricted build context. The crate depends
only on `node_graph`, `signal_processing`, serialization, and inventory support.

## Ownership boundaries

The API does not own a built-in node catalog, graph lowering, execution lifecycle, UI state,
viewer widgets, capture export, or target selection. Concrete node state migration remains beside
the concrete graph-node feature. The current directory-catalog path contract is a documented
boundary to move behind UI/platform ownership in the proposed architecture.

## Public contract namespaces

`node` is the supported feature-facing contract. It contains `RuntimeBuilder`,
`GraphNodeRegistration`, `PayloadRegistration`, `LiveCaptureFeature`, and
`CaptureGraphSourceFactory`. A feature implements or submits those contracts; it does not depend
on the compiler or application crate.

`node_support` supplies the values and narrowly scoped services required by those implementations:
`PortKind`, `PortValue`, resolved inputs, `NodeBuildContext`, node-owned state decoding, capture
identity and presentation descriptions, lane and decoder-table descriptions using stable renderer
keys, sampling-overlay input descriptions, and trigger/live-capture edits. The namespaces are
deliberately separate rather than root re-exports: features import traits from `node` and supporting
values from `node_support`.

## Builder context

`NodeBuildContext` is the only plugin-visible materialization service contract. It exposes derived
lane access, retention and persistent-cache configuration, and run-owned sampling-point storage.
A concrete clocked feature records accepted sampling decisions only after it applies its own edge
and qualifier semantics, or installs its own lazy provider that reconstructs those decisions from
indexed inputs. The compiler and viewer consume the neutral result and never learn protocol rules.

The compiler owns the concrete context state and its broader host-only result operations. Those
operations remain in the compiler facade, so a plug-in cannot receive or import concrete compiler
state through this API.

## Presentation contract

Node-supplied descriptions are UI-independent and remain distinct from compiler or UI results:

| API description | Consumer-owned result |
| --- | --- |
| `SamplingOverlayDescriptor` | compiler `SamplingOverlayCandidate` |
| `TriggerConfigurationFeature` | compiler `DiscoveredTriggerConfiguration` |
| `CapturePresentation` | compiler `DiscoveredCapturePresentation` |
| `DecoderTableColumnDescriptor` | UI decoder-table source |
| `LanePresentationDescriptor` | UI viewer lane group |
| `RuntimeBuilder` | compiler `CompiledNode` |
| `LiveCaptureFeature` | compiler `DiscoveredLiveCaptureFeature` |

The API has no `egui` or `logic_analyzer_viewer` dependency. Concrete features may contribute
renderers under stable keys, while generic contracts transport only those keys and neutral metadata.
