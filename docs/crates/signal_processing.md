# `signal_processing` Design

## Responsibility

`signal_processing` owns UI-independent generic signal execution, capture data, artifact storage,
derived-data storage, and capture-session contracts. It contains no concrete capture format,
device, protocol decoder, graph document, widget, or target selection.

## Facade and dependencies

Its crate root is the supported façade. The public `capture`, `live_capture`,
`live_capture_store`, `logic_analyzer`, `derived_word_store`, and `waveform_index` namespaces
name substantial generic domains; runtime plumbing is re-exported from the root. The crate has no
workspace dependency and therefore remains the lowest reusable logic-analyser layer.

## Ownership boundaries

`ProcessNode`, ports, channels, schedulers, pipeline managers, and work executors own generic
execution. Artifact repositories own immutable byte publication. Capture and waveform-index types
own raw-data access; derived stores own collected payload query and persistence; live-capture
types own driver-neutral acquisition and session state. Concrete implementations live above this
crate. Contract conformance and platform-parity tests belong here.

The proposed internal and eventual crate decomposition is defined in
[Crate Responsibility Design](../architecture/crate_responsibility.md).
