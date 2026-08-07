# Processing Graph Workflows

This document describes the runtime behavior that is distributed across graph registration,
lowering, execution, derived-data storage, and presentation. All workflows use generic capability
contracts; concrete nodes and payload plug-ins supply behavior without adding name-based cases to
the compiler, runtime, UI, or viewer.

The terms used in these sequences are defined in
[Vocabulary and Concepts](vocabulary_and_concepts.md).

## Building the processing graph

Before lowering, the UI converts its selected graph endpoints to an `OutputSubscriptionPlan`.
`GraphLowerer` then analyzes the editable document against one immutable `GraphRegistry` snapshot.
It does not allocate stores, prepare a source, create an execution manager, or start work.

```mermaid
sequenceDiagram
    actor User
    participant UI as logic_analyzer_ui
    participant Service as UiGraphService
    participant Lowerer as GraphLowerer
    participant Registry as GraphRegistry
    participant Semantics as GraphNodeSemantics
    participant Plan as ProcessingGraph

    User->>UI: Select outputs / edit graph
    UI->>UI: Persist stable node-output selections
    UI->>Service: set_output_subscriptions(plan)
    UI->>Service: validate, discover, or Run(graph)
    Service->>Lowerer: lower(GraphState)
    Lowerer->>Registry: resolve definitions and payload registrations
    Lowerer->>Lowerer: resolve reroutes and add generic collectors
    Lowerer->>Lowerer: retain the upstream closure of sinks and subscriptions
    loop Each retained connection
        Lowerer->>Semantics: offered kinds and connection contracts
        Lowerer->>Semantics: accepted kinds and connection contracts
        Lowerer->>Lowerer: choose first offered compatible kind
        Lowerer->>Semantics: resolve runtime input/output ports and buffer policy
    end
    Lowerer->>Lowerer: validate required inputs, cycles, and time-domain sources
    Lowerer->>Registry: obtain materializers and optional source/presentation metadata
    Lowerer->>Plan: assemble immutable nodes, edges, subscriptions, and overlays
    Plan-->>Service: execution-ready plan or NodeId-owned diagnostics
```

### Capability negotiation

Negotiation occurs per connection rather than per editor socket. The producer supplies an ordered
set of `PortKind`s and the consumer supplies the kinds it accepts. The first kind in the
producer's preference order that the consumer accepts wins. When both sides declare semantic
connection-contract identities, those sets must also overlap. The compiler then asks both sides
for the runtime port names for that selected kind.

Generated collectors accept only payload kinds with a `PayloadRegistration`. This ensures the
processing plan carries a type-erased ingestion adapter and a stable payload identity before the
runtime sees the plan. Compiler errors retain the owning `NodeId`, allowing the editor to badge the
node without interpreting diagnostic text.

Sampling and presentation discovery are also capability driven. `GraphNodePresentation` describes
which node inputs are the clock and sampled groups, which output supplies an optional retained-word
source, and which generic lane/table descriptors apply. The lowerer resolves those descriptions to
capture-row and producer-output identities and stores only the resolved values in the plan.

## Starting and running a pipeline

The UI's concrete `UiGraphService` owns a lowerer and a separate runtime. Pressing Run uses them in
sequence. A file/synthetic UI Run releases passive preview handles and clears the selected graph's
derived cache entries so Run means fresh execution. Re-analysis uses a dynamic capture-session
source and fresh derived stores. Other runtime clients can keep validated cache entries and allow
execution pruning.

```mermaid
sequenceDiagram
    actor User
    participant UI as logic_analyzer_ui
    participant Service as UiGraphService
    participant Lowerer as GraphLowerer
    participant Runtime as GraphRuntime
    participant Repo as ArtifactRepository
    participant Materializer as RuntimeMaterializer / payload catalog
    participant Manager as signal_runtime::AppManager
    participant Run as GraphRun and RunData

    User->>UI: Run
    UI->>UI: Refresh output plan and release cached preview
    opt No capture-session replay override
        UI->>Service: clear selected derived cache entries
    end
    UI->>Service: start_run(GraphState, context, source overrides)
    Service->>Lowerer: lower(GraphState)
    Lowerer-->>Service: ProcessingGraph
    Service->>Runtime: start(ProcessingGraph, context, overrides)
    Runtime->>Repo: configure keys and validate available stores
    Runtime->>Runtime: disconnect cached collector inputs and prune unreachable producers
    Runtime->>Materializer: build retained nodes in topological order
    Materializer-->>Runtime: ProcessNode
    Runtime->>Manager: add_node_deferred(NodeSpec, subscriptions)
    Runtime->>Manager: start_all_deferred()
    Runtime->>Run: publish lanes, subscriptions, overlays, diagnostics, readiness
    Run-->>UI: active GraphRun plus shared RunData handles
    loop Application frames
        UI->>Run: pump(budget), progress, diagnostics, readiness
        Run->>Manager: cooperative work or threaded status
        Run-->>UI: updated shared stores and snapshots
    end
```

All initial subscriptions are installed before the manager starts any node. This matters for
self-threading sources, which snapshot their subscribers when work begins. Native composition
injects a threaded manager; web composition injects a cooperative manager. The graph runtime uses
the same materialization and lifecycle contract for both.

When worker execution is selected, `logic_analyzer_graph_orchestration` transports the request to
a worker-owned composition. The worker calls its own `GraphLowerer`, passes the resulting plan to
its own `GraphRuntime`, and returns run messages. This preserves the same compiler/runtime boundary
without requiring the plan's in-process trait handles to cross the worker transport.

## Collecting derived data

Retention is represented by compiler-generated collector nodes. A collector has no concrete
payload branches: its plan-owned payload catalog selects the registered adapter for each negotiated
kind and builds a type-erased ingestor.

```mermaid
sequenceDiagram
    participant Producer as Concrete ProcessNode
    participant Channel as signal_runtime typed channel
    participant Collector as Generated DerivedDataCollector
    participant Catalog as ProcessingPayloadCatalog
    participant Adapter as PayloadAdapter / ingestor
    participant Lanes as DerivedLanes
    participant Repo as ArtifactRepository
    participant Runtime as GraphRuntime
    participant UI as logic_analyzer_ui

    Collector->>Catalog: descriptor and adapter for negotiated PortKind
    Catalog-->>Collector: stable payload contract and request customization
    Collector->>Adapter: create_ingestor(CollectedLaneRequest)
    Producer->>Channel: publish typed payloads
    Channel->>Collector: deliver retained payloads
    Collector->>Adapter: ingest in stream order
    Adapter->>Lanes: publish queryable lane generations
    opt Payload supports persistent indexed storage
        Adapter->>Repo: publish immutable data and index artifacts
    end
    Runtime-->>UI: RunData with lane and producer metadata
    UI->>UI: resolve renderer keys and bind waveform/table views
    UI->>Lanes: query only the visible range
```

`signal_derived` owns retained payloads, ingestion, indexes, sampling stores, and artifact formats.
`logic_analyzer_graph_runtime` owns when stores are configured and attached to a run.
`logic_analyzer_ui` owns which retained data is visible and how neutral renderer keys become
widgets. Presentation changes never alter collection policy.

## Derived-cache and sampling-point behavior

Persistent keys are protocol neutral. For a derived lane, the runtime hashes the cache ABI and
package version, collector state and member, selected port and payload kind, and a canonical hash
of the complete upstream graph. Each upstream node contributes its definition identity, canonical
state, incoming edges, and capture identity. A dynamic capture identity makes the result
non-persistent; a stable capture identity makes equivalent plans reusable.

Sampling overlays use either an existing retained-word lane as a lazy point provider or a dedicated
`SamplingPointStore`. Dedicated persistent stores use the same upstream identity rules as derived
lanes. Visibility is not part of either key.

```mermaid
sequenceDiagram
    participant Lowerer as GraphLowerer
    participant Runtime as GraphRuntime
    participant Repo as ArtifactRepository
    participant Node as Sampling node
    participant Store as SamplingPointStore
    participant UI as logic_analyzer_ui
    participant Viewer as logic_analyzer_viewer

    Lowerer->>Lowerer: resolve clock row, sampled rows, and retained-word source
    Lowerer-->>Runtime: ProcessingGraph with SamplingOverlayCandidate
    alt Candidate has a retained-word lane
        Runtime->>Store: install lazy provider over DerivedLanes
    else Stable upstream capture identity
        Runtime->>Repo: validate sampling cache key
        alt Node executes
            Runtime->>Store: create persistent store
            Node->>Store: record accepted sampling points
        else Upstream execution is cache-pruned
            Runtime->>Store: open persistent store
        end
    else Dynamic or non-capture upstream
        Runtime->>Store: use run-owned in-memory store
        Node->>Store: record accepted sampling points
    end
    Runtime-->>UI: resolved overlay and shared store
    UI->>Viewer: bind selected overlay rows
    Viewer->>Store: query points for visible time range
```

Opening a document uses `GraphRuntime::load_cached_data`: it validates derived stores, opens any
matching sampling stores, materializes only collector adapters needed to expose cached lanes, and
does not start producers or sinks. This passive preview is replaced when the user presses Run.

During live graph reconciliation, sampling stores are reused by stable graph `NodeId` before the
runtime diffs the old and new plans. Presentation-only edits therefore retain accumulated sampling
decisions. A run that pruned producers because of persistent cache hits requires a full restart for
structural edits, because the absent producer path cannot be safely reconciled in place.

Cache maintenance is best effort and correctness neutral. Only a validated hit disconnects a
collector input. A missing, corrupt, or unreadable entry leaves the producer connected so data is
regenerated. Cleanup receives the active keys as pins and cannot change the meaning of a run.

## Live graph reconciliation

The UI compares semantic graph snapshots, excluding editor layout and other presentation-only
state. A valid changed snapshot is lowered to another complete `ProcessingGraph`; the active run
then classifies the plan difference.

```mermaid
sequenceDiagram
    participant UI as logic_analyzer_ui
    participant Lowerer as GraphLowerer
    participant Run as LiveRun
    participant Manager as signal_runtime::AppManager

    UI->>UI: detect semantic document change
    UI->>Lowerer: lower(edited GraphState)
    alt Document is invalid while editing
        Lowerer-->>UI: diagnostics
        UI->>Run: leave active plan unchanged
    else Valid replacement plan
        Lowerer-->>UI: ProcessingGraph
        UI->>Run: apply_processing_graph(plan)
        Run->>Run: reuse sampling stores and diff by NodeId
        alt Hot configuration
            Run->>Manager: reconfigure node
        else Added or removed branch
            Run->>Manager: add or remove affected nodes
        else Restartable state or wiring change
            Run->>Manager: restart affected node
        else Source change or cache-pruned topology
            Run-->>UI: NeedsFullRestart; active run remains available
        end
    end
```

Configuration epochs use the same lowering path but accept only materializer-declared hot
configuration. The run schedules those changes at the supplied sample/time boundary; structural,
source, and acquisition changes remain in the editable document and are reported as deferred.
