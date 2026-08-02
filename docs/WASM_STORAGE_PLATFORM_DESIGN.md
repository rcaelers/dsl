# Unified Native and Web Storage Platform Design

## Current architecture

Raw-capture storage, finite and growing waveform indexes, finalized replay, decoded-word storage,
viewer lanes, compiler data models, concrete graph-node definitions, and source/sink factory
contracts are platform-neutral. These facilities use repository-backed stores on every target and
contain no filesystem operations or target-selected source.

The injected artifact repository selects only the byte backing. Native composition supplies a
durable adapter whose immutable reads are mmap-backed. Web composition supplies the portable
process-lifetime memory repository, which stores artifacts in configurable chunks under a
configurable byte budget. Ranges within one memory chunk are borrowed directly; cross-chunk ranges
use the same owned-region fallback as any other repository without stable contiguous storage.
Both adapters publish immutable generations atomically.

The authoritative capture store publishes one bounded packed-sample artifact per committed chunk,
followed by a manifest generation that makes the chunk visible. Live cursors observe only the
committed prefix. Finalization seals that same generation for replay; it does not copy the capture
into another representation. Session metadata, retention plans, and application presentation
metadata are repository artifacts addressed by the capture-session identity. Cleanup, pinning,
recovery, and prefix reclamation operate on those identities rather than directories or paths.

Finite waveform indexes publish one bounded leaf artifact per `(channel, block)` and publish their
compact root directory last. Growing indexes publish fixed-size summary pages. Exact queries cache
individual packed raw blocks as artifacts and retain the repository's immutable byte backing, so a
native mmap and an owned-memory region execute the same query code. These opportunistic raw-block
cache publications are atomic but omit the native durability barrier; they are validated by
identity and can always be reconstructed from the authoritative capture. Derived cache blocks use
the same policy because they are not discoverable without the final index and manifest. Authoritative
capture chunks, waveform indexes, derived indexes, and manifests explicitly flush before publication.
No capture, raw cache, waveform index, or replay session must fit in one resident allocation.

Host source acquisition and output destinations are selected in `logic_analyzer_platform`.
Native composition injects DSL and Sigrok path adapters, filesystem-backed writer storage, the
U3Pro16 USB transport and FPGA-image provider, and repository-backed capture export. Web composition
injects explicit unavailable file, writer, USB, and export capabilities. The portable Sigrok source
supports explicitly configured demo data; web composition does not silently substitute it for a
file. Browser file/blob handles and browser download destinations are optional future adapters.

Platform selection occurs at complete implementation-file boundaries in `logic_analyzer_platform`.
Generic compiler, runtime, viewer, graph-node, and UI code contains no target-selected source. The
temporary processing exceptions are the U3Pro16 native device-runtime leaves and isolated DSL and
Sigrok path-compatibility leaves; archive parsing and prepared-source execution remain portable.

Stores address every encoded block, index, and manifest with typed artifact keys. A block
is published before its directory entry becomes visible, and a persistent manifest is published
last. Missing manifests are cache misses; invalid manifests, indexes, or block generations are
rejected and invalidated as a unit. Unfinished ephemeral artifacts are reclaimed when their last
store handle is dropped. Compiler cache lookup, graph pruning, preview, invalidation, and cleanup
apply the same policy to durable native and process-lifetime web repositories.

`logic_analyzer_platform` currently composes the UI host-service port and selects the artifact
repository contract. Native and web application bootstraps obtain an opaque `PlatformServices`
bundle from that crate and inject its UI services when constructing the application. That service
boundary passes the selected repository through the graph service to every compiler run and
`NodeBuildContext`; concrete derived-lane configuration therefore receives a capability rather than
selecting a target backend. The native adapter provides a durable repository whose same-directory
publication is atomic and whose immutable reads use mmap-backed byte regions. The web adapter
selects the portable process-lifetime memory repository. The native adapter also owns file dialogs,
graph document I/O, and
allocation of the application directory backing its repository. Cache administration is common
compiler policy over the injected repository, so ephemeral web caches use the same identity,
preview, pruning, invalidation, inspection, and cleanup paths. The native adapter also owns native
configuration-file discovery and I/O. It then passes decoded portable settings and bindings
to the UI. It supplies
optional system symbol fonts; the UI owns bundled fallback fonts and the portable font installation
algorithm. The web adapter exposes unavailable direct-file operations and supplies embedded settings.
Finite-source preparation uses the compiler-owned execution contract: the native platform adapter
uses a bounded worker, while the web adapter selects the portable inline executor. The compiler
polls one task contract and contains no target-selected source-preparation implementation.
The application-runtime facade likewise receives a factory from platform composition. Native runs
receive the threaded pipeline-manager backend; web runs receive the portable cooperative backend.
The compiler creates managers through the same factory contract and does not select either backend.
Portable processing work uses the `signal_processing::WorkExecutor` contract. The platform service
bundle supplies bounded finite-work execution and host-owned long-running task execution through
one capability, passes it through the UI graph-service construction boundary, and the compiler makes
it available to node builders in their `NodeBuildContext`. Concrete nodes choose whether they need
finite or long-running work without selecting a target or a platform implementation.
The platform service bundle also supplies the UI-owned `CaptureExportService`; its native adapter
owns repository access and export-worker execution, while the portable unavailable service reports
absence explicitly. The UI has no export feature flag, target-selected export module, or concrete
export dependency. Graph document persistence and embedded node file dialogs likewise cross
host-service contracts, and the node-graph widget exposes only model snapshots and replacement.
Native shell integrations exchange portable commands and UI state through that service contract;
their queues and repaint wake-ups remain inside the platform adapter. Runtime cache diagnostics use
the same adapter boundary and one portable UI snapshot path. Embedded graph-node file controls use
the portable `node_graph::FileDialogService`, supplied through the same platform service bundle;
the widget contains no target selection or native dialog dependency. The UI owns the portable request,
result, and service contract and does not select an implementation.

## Unified native and web data plane

Native and web builds use the same capture buffering, block encoding, indexing, cache planning,
cache lookup, query, and eviction algorithms. Platform implementations provide only the host
capabilities required to acquire resources, retain bytes, execute work, and communicate with
devices. The absence of a durable browser repository or USB transport changes advertised
capabilities, not the processing data model.

The design applies to:

- packed raw-capture blocks and their waveform indexes;
- finalized and growing live-capture repositories;
- indexed derived payload stores;
- decoded-block and raw-block memory caches;
- compiler cache discovery, validation, pruning, and publication;
- finite-source preparation and viewer attachment.

Browser file import, browser file export, OPFS persistence, Web Workers, and WebUSB are optional host
adapters. The shared data plane does not depend on their availability.

### Goals

- Execute one codec, index builder, query implementation, and cache policy on every target.
- Compile the same source files in every reusable core crate on native and web targets.
- Make mmap and owned heap memory interchangeable byte backings.
- Keep persistent and ephemeral storage behavior behind one artifact-repository contract.
- Keep browser promises, permission prompts, JavaScript handles, and native paths outside parsers,
  indexes, graph nodes, the compiler data model, and the viewer.
- Permit cooperative execution today and native or browser parallel execution without duplicating
  algorithms.
- Keep persisted offsets, lengths, sample counts, and timestamps ready for 64-bit address spaces.
- Make unavailable capabilities explicit and testable rather than replacing production behavior
  with platform-specific application semantics.

### Non-goals

- Emulating a native hierarchical filesystem in generic code.
- Requiring persistent browser storage before the common in-memory backend is usable.
- Requiring browser file drag-and-drop or WebUSB for storage unification.
- Giving widgets direct access to paths, browser handles, USB devices, or cache repositories.
- Making every low-level storage handle transferable between native threads or browser workers.
- Selecting platform behavior from node names, port names, payload names, or protocol identifiers.
- Treating matching public APIs over separate native and web implementations as sufficient parity.

### Capability model

Platform support is expressed through orthogonal capabilities rather than one target-shaped
`Platform` interface. Capability contracts belong to the core crate that owns the behavior.
Reusable host implementations and target selection belong to `logic_analyzer_platform`. A host can
provide any useful combination.

| Capability | Native implementation | Initial web implementation | Optional web implementation |
| --- | --- | --- | --- |
| Prepared random-access input | file reader or mmap | embedded or owned chunked bytes | user-selected `File`/`Blob` materialized or accessed by a worker |
| Artifact repository | directory, files, locks, atomic rename | process-lifetime memory repository | Origin Private File System (OPFS) |
| Immutable byte region | mmap-backed range | `Arc<[u8]>`-backed range | transferred worker-owned `ArrayBuffer` chunks |
| Work execution | bounded native worker pool | cooperative application pump | dedicated Web Worker or worker pool |
| USB transport | native USB backend | unavailable capability | WebUSB backend where supported and permitted |

Consumers query capability properties such as durability, writable capacity, concurrent-reader
support, and available parallelism. They do not branch on `wasm32`, browser names, operating-system
names, or a storage implementation name.

Portable implementations remain in their behavioral owner and compile everywhere. This includes
the chunked-memory repository, owned byte backing, deterministic fake sources, and cooperative
executor. `logic_analyzer_platform` selects or constructs them but does not fork their algorithms.
Persistent metadata receives the root-level `signal_processing::UnixTimeSource` capability. Its
default implementation uses `web-time` on every target, while deterministic conformance fixtures
inject a fixed clock so complete manifests and encoded generations can be compared byte for byte.

### Platform adapter crate

`logic_analyzer_platform` is a top-level adapter crate, not a lower-level dependency of the core.
It depends inward on the crates that define capability contracts and returns implementations of
those contracts to the application composition root. It does not define parallel copies of their
data models.

Its private layout has one target-selection point:

```text
logic_analyzer_platform/
  src/
    lib.rs                    curated crate-root construction API
    services.rs               composition-only adapter bundle
    platform/
      mod.rs                  the only reusable target selector
      native.rs               native host-service composition
      native_sigrok/
        mod.rs                embedded-Python Sigrok host composition
        discovery.rs          package discovery and native directory scanning
        execution.rs          Python-backed Sigrok execution factory
        catalog.rs            persisted external-decoder catalog adapter
      web.rs                  web host-service composition
```

The crate root exposes constructors and an opaque composition bundle, not public `native` and
`web` namespaces. The application bootstrap decomposes that bundle into the capability objects
accepted by the compiler, runtime, processing factories, dynamic node catalogs, and UI host. No
core crate names `PlatformServices` or depends on `logic_analyzer_platform`.

Traits implemented by the adapter crate are supported cross-crate ports re-exported from the crate
root of their behavioral owner. For example, storage and execution ports belong to
`signal_processing`, cache-administration ports belong to `logic_analyzer_graph_compiler`, embedded
node-control dialogs belong to `node_graph`, and application dialogs, host commands,
cache diagnostics, and capture export belong to `logic_analyzer_ui`.
Making those ports implementable does not expose their concrete native or web dependencies. The
capture-export port has one target-neutral contract, and its repository-backed adapter is selected
in `logic_analyzer_platform`. The storage-contract work replaces the remaining native session
repository behind that adapter without changing the UI contract.

The Sigrok decoder node follows the same split. `logic_analyzer_processing` owns the portable
decoder configuration, state machine, output contracts, and `SigrokExecutionFactory` port.
`logic_analyzer_graph_nodes` owns the portable graph-node schema and turns portable discovery
snapshots into node templates. `logic_analyzer_platform::platform::native_sigrok` owns Python
interpreter initialization, the `sigrokdecode` compatibility host, package discovery, directory
settings, and the native execution-factory implementation. A host injects that factory and
scanner; a host without an embedded runtime injects no implementation and the portable node
reports that the capability is unavailable.

The memory repository, owned backing, fake source, cooperative executor, and other host-independent
implementations remain in their existing owner crates and can be selected on native, web, or in
tests. `logic_analyzer_platform` contains only code whose implementation actually calls a host API
or establishes target-specific execution.

Target-specific dependencies such as `memmap2`, native dialog libraries, native USB libraries,
embedded-runtime libraries, `wasm-bindgen`, and `web-sys` are declared only by
`logic_analyzer_platform` or a bootstrap crate. An explicitly allowlisted processing adapter may
temporarily retain a concrete format or device dependency, but generic core crates do not acquire
that dependency transitively.

### Source-code parity

For reusable core crates, native/web parity means the same source code, not only equivalent public
types. Specifically, those crates have:

- one module tree with no target-selected implementation path;
- no `#[cfg(target_arch = ...)]`, inverse target gate, target `cfg_attr`, or target inspection
  through `cfg!`;
- no target-specific dependency section in their manifest;
- no native-only or web-only field, variant, function, trait method, registration, or re-export;
- one graph-node definition, state schema, builder, migration path, and processing contract;
- one algorithm for buffering, encoding, indexing, caching, querying, ordering, and source
  preparation;
- runtime behavior selected only from injected capabilities or explicit user configuration.

This rule covers production, component-test, benchmark, and example source within the core crate.
Target-specific test harness entry points and browser/native conformance composition belong to the
platform or workspace integration-test package. A portable test fixture does not use target gates
to change its expected behavior.

An unavailable capability is represented by an injected unavailable provider or an absent optional
provider. The core executes the same error and diagnostic path on every target. It does not compile
a different node, silently generate data, or discard output based on the target.

### Layering

Resource acquisition and data processing are separate phases:

```text
user gesture / application configuration
                 |
                 v
       host resource acquisition
        path, File, Blob, USB permission
                 |
                 v
    logic_analyzer_platform adapter
       implements owner contracts
                 |
                 v
       prepared capability handle
     source / repository / transport
                 |
                 v
      shared processing data plane
  codec, indexes, caches, queries, policy
                 |
                 v
      platform-neutral query handles
                 |
                 v
         compiler and viewer
```

Browser acquisition is asynchronous because it may require a user gesture or a Promise. Shared
parsers and indexes receive only a prepared algorithm-facing handle after acquisition completes.
They never initiate a picker or wait for permission.

### Prepared random-access input

A finite capture parser consumes an opaque random-access byte source instead of a `PathBuf`.
The contract describes behavior and stable identity, not where bytes live:

```rust
trait RandomAccessReader {
    fn len(&self) -> Result<u64, SourceReadError>;
    fn read_at(&mut self, offset: u64, destination: &mut [u8])
        -> Result<usize, SourceReadError>;
}

trait PreparedByteSource {
    fn identity(&self) -> SourceIdentity;
    fn capabilities(&self) -> SourceCapabilities;
    fn open_reader(&self) -> Result<Box<dyn RandomAccessReader>, SourceReadError>;
}
```

`SourceIdentity` is an opaque stable fingerprint used by cache keys. It is not a display name or a
filesystem path. Native files can derive it from validated metadata and content fingerprints.
Owned web bytes can derive it from their length and content. A future browser file adapter can add
host-provided identity hints, but validation remains content-safe.

`RandomAccessReader` is deliberately a reader session rather than a globally shareable handle.
Native parallel indexing can open a reader per worker. A browser worker can own a JavaScript or OPFS
handle that is neither transferable nor safe to share. Algorithms operate on the reader they are
given and do not require the underlying host object to implement native threading traits.

A source advertises whether it supports independent readers and efficient random access. The work
planner limits concurrency when the source supplies only one reader. Short reads and source changes
are explicit errors; parsers never assume a single read fills the requested range.

Published repository artifacts can also be exposed as prepared byte sources. The adapter opens a
fresh immutable artifact generation for every reader. When a repository cannot expose a stable
physical byte region, the shared range helper fills an owned immutable region through `read_at`;
callers do not branch on mmap availability.

### Artifact repository

Generated raw blocks, indexes, derived blocks, directories, and manifests use a logical artifact
repository. Generic code addresses artifacts by typed keys, not paths:

```rust
trait ArtifactRepository {
    fn capabilities(&self) -> RepositoryCapabilities;
    fn namespaces(&self) -> Result<Vec<ArtifactNamespace>, RepositoryError>;
    fn open(&self, key: &ArtifactKey) -> Result<Option<Box<dyn ReadArtifact>>, RepositoryError>;
    fn begin_write(&self, key: &ArtifactKey) -> Result<Box<dyn WriteArtifact>, RepositoryError>;
    fn remove(&self, key: &ArtifactKey) -> Result<(), RepositoryError>;
    fn entries(&self, namespace: &ArtifactNamespace)
        -> Result<Vec<ArtifactMetadata>, RepositoryError>;
}

trait ReadArtifact {
    fn len(&self) -> Result<u64, RepositoryError>;
    fn read_at(&mut self, offset: u64, destination: &mut [u8])
        -> Result<usize, RepositoryError>;
    fn region(&self, range: ByteRange) -> Result<Option<ByteRegion>, RepositoryError>;
}

trait WriteArtifact {
    fn write_at(&mut self, offset: u64, source: &[u8]) -> Result<(), RepositoryError>;
    fn truncate(&mut self, len: u64) -> Result<(), RepositoryError>;
    fn flush(&mut self) -> Result<(), RepositoryError>;
    fn publish(self: Box<Self>) -> Result<(), RepositoryError>;
}
```

The actual Rust API may split administrative and hot-path handles further, but it preserves these
semantics:

- a writer creates an unpublished artifact;
- authoritative data, indexes, and manifests are flushed before publication; explicitly
  rebuildable cache blocks may omit the durability barrier while retaining atomic publication;
- publication makes one validated generation discoverable;
- incomplete artifacts are not cache hits;
- readers observe an immutable published generation;
- removal and cleanup operate on typed cache identities;
- repository errors distinguish unavailable, exhausted quota, permission loss, I/O failure,
  corruption, and unsupported optional behavior.

The native repository adapter is an isolated leaf in `logic_analyzer_platform`; it implements
publication with files and atomic filesystem operations. The platform-independent memory repository
in `signal_processing` keeps published artifacts in bounded, chunked process-lifetime memory and
can be selected on any target. Both implementations satisfy the same lifecycle and prepared-source
conformance fixture. A future OPFS adapter in `logic_analyzer_platform` can add web durability
without changing store, compiler, or viewer behavior.

Durability is a repository capability. A cache requested on an ephemeral repository is still a
real cache for the current application lifetime: it uses the same keys, validation, graph pruning,
and eviction policy, but it cannot produce a hit after the page is reloaded.

### Immutable byte regions

Codecs and queries consume `ByteRegion`, which owns a range of an immutable backing and exposes a
borrowed byte slice for the duration of an operation. The backing is private to
`signal_processing`.

```rust
struct ByteRegion {
    backing: Arc<dyn ByteBacking>,
    offset: usize,
    len: usize,
}

trait ByteBacking {
    fn bytes(&self) -> &[u8];
    fn shares_backing(&self, other: &dyn ByteBacking) -> bool;
}
```

The native repository adapter supplies mmap-backed regions when possible. Portable memory
repositories supply `Arc<[u8]>`-backed regions for ranges within one chunk. A repository that
cannot expose a stable region for a requested range returns `None`, and the common reader fills an
owned region through `read_at`. Mmap therefore
remains an optimization supplied by `logic_analyzer_platform`, not a different storage or indexing
model inside `signal_processing`.

Large artifacts are chunked. Neither native nor web code requires one artifact, one capture, or one
index to fit in a single allocation or `usize` range.

### Shared encoded stores and indexes

The derived-word block codec, directory format, presence summaries, exact-window queries,
nearest-boundary queries, decoded-block LRU, and integrity validation are the only implementations.
They read and write artifacts through the repository and byte-region contracts.

The web memory repository stores the same encoded blocks, indexes, and manifests as the native repository.
It does not retain a platform-specific `Vec<Word>` as its authoritative representation. Small
captures naturally use one or a few blocks; they do not select a different query engine.

### Cache planning and compiler behavior

The compiler owns target-independent cache planning:

- derive cache keys from graph, node state, payload identity, source identity, and schema versions;
- ask the repository which validated generations are available;
- prune producer branches satisfied by cache hits;
- attach cached preview lanes;
- invalidate selected outputs when Run requests fresh execution;
- publish completed generations;
- apply size and age policy while respecting pinned active generations.

A platform adapter supplies repository discovery and cleanup operations. It does not replace the
planning algorithm with no-ops. An ephemeral web repository can therefore satisfy repeated runs or
graph changes during one application session even before browser persistence exists.

Compiler IR and saved documents contain storage intent and stable identities, not native paths,
mmap flags, browser handles, or target-specific variants. Saved-graph migrations remain explicit
and user-visible when the portable schema changes.

### Source preparation

Finite-source preparation uses one state machine on every target:

```text
Configured
    |
    v
Acquire or resolve prepared source
    |
    v
Read and validate source metadata
    |
    v
Open valid cached raw/index artifacts, or build them
    |
    v
Publish CaptureDataSource and readiness metadata
```

The state machine delegates source reads, artifact construction, and work scheduling through
capabilities. A missing browser picker or filesystem does not require another state machine: the
source is either already embedded/prepared or acquisition reports that the capability is
unavailable.

Source preparation owns long-running progress, cancellation, and generation replacement. The UI
requests an operation and renders its state; it does not parse files, build indexes, or manage
storage.

Native file-source factories resolve configured paths into host-owned `PreparedByteSource`
instances before constructing a processing source or deferred viewer index. The native adapter
validates the acquired file stamp whenever it opens or reads a session and reports replacement as
`SourceChanged`. DSL and Sigrok archive readers adapt an acquired `RandomAccessReader` to the ZIP
container interface; their metadata parsers, capture readers, and index factories do not open a
path. Fresh reader sessions preserve parallel-indexing capability without sharing a native file
cursor.

The compiler assigns a monotonically increasing generation to each configured source preparation.
Its observable snapshot contains the generation, readiness state, and latest index progress.
Replacing, clearing, failing, or explicitly resetting the source cancels the active control before
discarding its task. Progress callbacks return whether work may continue, so capture-index builders
stop at a deterministic work boundary after cancellation. Completion is published only from the
task retained by the active generation; stale workers cannot replace current viewer data.

The preparation algorithm is identical for inline/cooperative and native execution. The host
executor only decides where the capability-driven operation runs. Cache validation, index build and
atomic index publication remain in the shared capture-index implementation behind the deferred
`CaptureIndexFactory` contract.

### Execution

Execution is a separate platform capability because native blocking threads are not available to
`wasm32-unknown-unknown`. Shared algorithms submit finite work units and long-running stream tasks
through one platform executor. The platform decides when and where those units run.

The executor contract provides:

- advertised parallelism for finite work;
- bounded finite-work submission and backpressure;
- host-owned long-running task execution for stream readers and runtime supervision;
- completion polling and waiting;
- deterministic output ordering independent of completion order.

The native executor adapter in `logic_analyzer_platform` uses a bounded worker pool for finite
work and host-created tasks for long-running readers and runtime supervision. The portable
cooperative executor compiles on every target and is the explicit fallback when the web host cannot
provide parallel workers. `signal_processing::WorkerMessage` carries owned operation identifiers,
sequence numbers, payloads, progress, cancellation, completion, and failure across a worker
boundary. A Web Worker adapter in `logic_analyzer_platform` dispatches registered operations with
those messages and returns owned result chunks. It does not attempt to send Rust closures, trait
objects, mmap handles, or borrowed slices between workers.

`signal_processing::portable_worker_kernels` registers the finite derived-word block encoder and
capture-index leaf builder under stable, versioned operation identifiers. Their compact binary
payloads use fixed-width coordinates and own all words or packed samples consumed by the operation.
Derived-word results are the same encoded blocks used by the persistent store; capture-index results
contain the same hierarchy data written by the index writer. Source readers, block builders, files,
repositories, and publication state never cross the worker boundary.

Capture-index scheduling reads at most the executor's advertised parallelism worth of raw blocks
ahead. It then merges independently completed leaves by channel and block before applying boundary
transitions and publishing them. Derived-word publication likewise commits completed encoded blocks
by sequence. Worker completion order therefore cannot change either persistent format or query
results, and the cooperative and parallel paths execute the same kernels. Index and derived-store
manifests are published only after all ordered results succeed; cancellation or failure leaves no
partially published generation for a later reader to accept.

`logic_analyzer_platform::WebWorkerAdapter` owns a bounded pool of browser workers and is constructed
with the absolute URLs of the generated JavaScript module and WASM binary. Worker construction
validates that the browser accepts the host mechanism, while module import and initialization remain
lazy until the first accepted operation. Each worker then initializes the same generated module and
invokes the exported portable-kernel entry point. The adapter keeps at most one running operation in
each worker and applies backpressure once its bounded host queue is full.

Request payloads are copied once from WASM memory into an owned `ArrayBuffer` and transferred to the
worker. Completed payloads use a standalone transferable buffer rather than attempting to detach
WASM linear memory. Sequence identifiers cross JavaScript as decimal strings so wasm32 does not
truncate them through the JavaScript number representation. Requests must use monotonically
increasing sequence identifiers.

Workers report readiness, unit progress, completion, operation failure, and bootstrap failure. The
adapter forwards progress immediately but buffers terminal messages until every earlier submitted
sequence has a terminal result. Cancellation removes queued work immediately and suppresses results
from already-running synchronous kernels; it does not claim to preempt a kernel while JavaScript is
executing it. A failed worker rejects its active request, remaining workers continue draining the
queue, and loss of the complete pool rejects all queued requests. Dropping the adapter terminates
the pool and releases its JavaScript callbacks.

`signal_processing::WorkerOperationQueue` owns this bounded scheduling state independently of the
host transport. Host readiness and results enter as portable events; runnable and cancellation work
leaves as `WorkerHostCommand` values. Native and Web Worker adapters translate those commands
without reimplementing queue policy. The queue's native and wasm conformance suite covers every
portable message variant, operation validation, backpressure, out-of-order completion, queued and
active cancellation, mismatched results, partial and complete worker-pool failure, and observable
parity with `CooperativeWorkerOperationExecutor`.

The native finite-operation adapter in `logic_analyzer_platform` runs those commands in a bounded
worker pool. Each slot owns one request channel and executes at most one operation at a time; the
shared queue bounds accepted work to four requests per advertised worker. The adapter converts a
kernel panic or transport loss into an ordered failure. Cancellation publishes an ordered failure
immediately and suppresses any late result, without claiming to preempt synchronous native code.
Closing the adapter disconnects the request channels, so idle workers exit without making shutdown
wait for a defective finite kernel.

`WorkerOperationExecutor` is the target-independent finite-operation host contract. Its capability
snapshot reports the selected cooperative or parallel mode, advertised parallelism, registered
operation identifiers, and the reason parallel execution is unavailable. It is deliberately
separate from `WorkExecutor`: ordinary closures, runtime nodes, stream readers, and watchdog tasks
remain on their existing cooperative or native host paths.

The web application passes the absolute generated-module URLs into the platform composition root.
`logic_analyzer_platform` derives a bounded worker count from browser hardware concurrency and
selects `WebWorkerAdapter` only after browser worker construction succeeds and every required
operation identifier is present in the portable registry. Failed construction or missing worker
configuration selects `CooperativeWorkerOperationExecutor`, which invokes the same registry and
emits the same progress and terminal messages. Graphs and processing nodes therefore do not branch
on worker availability, while diagnostics can distinguish parallel execution from its explicit
cooperative fallback. `AppServices` retains the selected generic executor for the application
lifetime; the UI neither owns browser-worker construction nor inspects the concrete adapter.

`AcquisitionContext` carries the selected executor into concrete live providers. Buffered and
streaming device captures therefore retain their portable lifecycle, cancellation, and backpressure
behavior while the host controls how their long-running acquisition task executes.

The execution contract separates the reusable bounded finite-operation queue from long-running
runtime supervision and file-source delivery so blocked stream endpoints cannot starve indexing.
Host adapters select their transport by platform composition rather than target conditionals in
shared algorithms. The WASM application build validates the inline worker bootstrap as JavaScript,
checks the generated browser module, and requires the exported portable-kernel entry point.

Worker transfers move ownership of large `ArrayBuffer` values where possible. Shared-memory worker
execution is optional because it requires a suitable browser and deployment isolation policy.
Correctness and file-format parity do not depend on shared memory.

### Browser persistence

OPFS is the preferred future durable browser repository because it provides origin-private random
access storage and worker-only synchronous access handles. It remains quota-managed browser data:
the browser or user can evict it, clearing site data removes it, and it is not a user-visible file
tree.

The OPFS adapter in `logic_analyzer_platform` runs storage operations in its owning worker. The main
application exchanges artifact keys, operation requests, progress, and owned byte chunks with that
worker. Generic code does not expose `FileSystemHandle`, Promise, or JavaScript error types.

An OPFS cache reports durability only for its current origin and successful storage grant. Cache
miss and eviction remain normal outcomes. User-selected files are source inputs and are not
silently treated as durable application cache entries.

### Browser file import and export

Browser file import is a proposed optional `logic_analyzer_platform` acquisition adapter. A picker
or drag-and-drop event produces a `File`, `Blob`, or file handle under a user gesture. The adapter
validates it and creates a `PreparedByteSource`. The first implementation may materialize small
files into chunked memory; larger-file support can place input in an owning worker or copy it to
OPFS before indexing.

Browser export is a separate destination adapter. Cache publication never triggers a download, and
download/export destinations are not used as internal artifact repositories.

These features do not block the shared memory repository, codec, index, cache-policy, or query
work.

### USB transport

The U3Pro16 device protocol owns a `UsbTransport` capability contract for control transfers, bulk
transfers, optional queued bulk reads, cancellation, close, and negotiated link speed. Its
`DsLogicU3Pro16TransportFactory` opens an opaque transport for the device protocol. The native
adapter in `logic_analyzer_platform` owns discovery, runtime-device validation, interface claiming,
the `rusb` dependency, and asynchronous libusb receive requests. The protocol, firmware upload,
FPGA configuration, acquisition planning, and packet state machines remain in
`logic_analyzer_processing` and depend only on the contract.

A future WebUSB adapter implements this same contract in `logic_analyzer_platform` and translates
browser permission and promise state into explicit capability and transport errors. A host without
such an adapter injects the unavailable U3Pro16 source factory rather than a synthetic live source.

WebUSB is an optional lower-priority adapter because browser support is not universal and device
interface access must be verified with the real hardware. Its absence does not produce a synthetic
hardware source pretending to provide live acquisition.

### Wasm32 and wasm64 data model

All persistent and cross-boundary quantities use fixed-width types:

- byte offsets, artifact lengths, sample counts, word counts, timestamps, and durations use `u64`;
- format fields never serialize `usize` or pointer widths;
- a `u64` becomes `usize` only after a checked conversion for one currently resident slice;
- ranges are checked for overflow before allocation or slicing;
- block directories and cache accounting do not assume one allocation can address an artifact;
- messages between browser workers use fixed-width serialized fields.

This keeps the formats portable to 64-bit WebAssembly without requiring wasm64 to be available now.
Wasm32 remains constrained by its address space and browser memory policy, so large captures require
bounded blocks and a repository rather than preloading one contiguous buffer.

### Ownership

- `signal_processing` owns the prepared-byte-source, artifact-repository, byte-region, execution,
  capture-store, index, derived-store, cache-format, and query contracts and their shared
  algorithms. Its chunked-memory repository, owned byte backing, deterministic fakes, and
  cooperative executor are portable implementations compiled unchanged on every target. It has no
  target selector or host dependency.
- `logic_analyzer_processing` owns concrete capture parsers, processing nodes, sinks, the U3Pro16
  device protocol, and portable format behavior. Parsers consume prepared byte sources; the
  U3Pro16 protocol consumes its injected USB transport contract. A complete file-I/O adapter leaf
  may remain here temporarily only when it is explicitly allowlisted and separating it would move
  concrete format behavior into the platform crate. Node state, factories, and protocol logic are
  not target-selected.
- `logic_analyzer_graph_nodes` owns concrete node state and builders. It passes platform-neutral
  source, destination, and device requests to processing facades and compiles the same node catalog
  code on every target.
- `logic_analyzer_graph_compiler` owns source-preparation orchestration, generic cache planning,
  execution lifecycle, collected outputs, and saved-document synchronization. It depends on
  injected capabilities rather than physical storage implementations and has no target-selected
  implementation modules.
- `logic_analyzer_ui` owns application-facing commands and status. Native and web application
  crates provide only thin bootstraps that install host capability adapters.
- `logic_analyzer_viewer` consumes capture and derived query handles. It has no repository,
  mmap, path, browser handle, cache administration, memory-management responsibility, or
  target-selected worker implementation.
- `logic_analyzer_platform` is an adapter/integration crate above the contract owners. It contains
  reusable native and web implementations for files, mmap, worker execution, browser handles,
  OPFS, dialogs, export destinations, and USB transport. It owns no codec, index, cache policy,
  graph policy, viewer behavior, concrete protocol, or node schema.
- `app_native` and `app_web` are unavoidable target-specific composition roots. They initialize
  their host, construct `logic_analyzer_platform` services, and inject them. They contain no
  reusable storage, processing, or scheduling implementation.

The dependency direction points from adapters to contract owners:

```text
signal_processing       logic_analyzer_processing       logic_analyzer_ui
        ^                         ^                            ^
        +-------------------------+----------------------------+
                                  |
                    logic_analyzer_platform
                                  ^
                                  |
                       app_native / app_web
```

Core crates never depend on `logic_analyzer_platform`; doing so would reverse the injection
boundary and reintroduce target selection into the shared data plane.

### Errors and diagnostics

Capability boundaries translate host errors into stable domain errors while retaining a diagnostic
source chain for the log panel. User-visible context identifies a capture source, graph node,
operation, or application service rather than an implementation module.

Expected capability outcomes include:

- unavailable on this host;
- permission required, denied, or lost;
- source changed during preparation;
- quota or configured memory budget exhausted;
- incomplete or short read/write;
- artifact corrupt, stale, incomplete, or version-incompatible;
- work cancelled;
- worker or device disconnected.

An unavailable optional capability does not become generic I/O failure and does not silently select
synthetic data. Demo nodes explicitly request deterministic generated sources.

### Testing contracts

Every storage and execution implementation is tested through shared conformance suites.

- `logic_analyzer_test_support` owns the reusable repository, capture-store, and derived-store
  fixtures. They accept only core traits, exercise deterministic inputs and clocks, and return
  comparable observations without knowing whether the backing is memory, files, or mmap.
- The in-memory repository suite runs as an ordinary unit test without filesystem dependencies.
- Native file and mmap repositories run the same artifact lifecycle, corruption, publication,
  cleanup, and query fixtures in isolated temporary directories.
- Native file-backed and in-memory stores produce byte-identical manifests, block directories, and
  encoded payload blocks for deterministic input.
- All backends return identical exact windows, presence windows, nearest boundaries, growing-prefix
  visibility, finish behavior, and cancellation outcomes.
- Cache-planning tests inject repository hits, misses, corruption, quota exhaustion, and cleanup
  outcomes; they do not require concrete storage.
- Source-preparation tests inject readers, repositories, and executors and cover short reads,
  source replacement, cancellation, cached preview, and fresh Run behavior.
- Executor tests verify bounded in-flight work, stable output ordering, cancellation, and failure
  without publishing partial generations.
- Fixed-width format tests exercise values above the wasm32 `usize` range without allocating those
  ranges.
- Wasm compilation and `wasm-bindgen-test-runner` tests verify that core crates compile one module
  tree without target selection and execute the common storage, query, node-contract, worker
  ordering, and cancellation behavior without native symbols.
- Future OPFS, file-import, worker, and WebUSB tests use repository-owned deterministic fixtures;
  hardware tests remain explicitly ignored and require an attached device.

### Platform selection

Reusable target selection is confined to one private selection module in
`logic_analyzer_platform`. That crate owns complete adapter implementations for:

- native file and mmap repository adapters;
- OPFS repository adapters;
- native and browser-worker executors;
- native dialogs and browser acquisition/export adapters;
- native USB and WebUSB transports;
- application host integration.

The native and web application crates retain only their required entry points and bootstrap APIs.
An explicitly documented complete file-I/O or USB leaf adapter in
`logic_analyzer_processing` is the sole temporary reusable-crate exception. Such an adapter contains
only host access; its node state, builder, parser or device protocol, and runtime contract remain
portable.

`signal_processing`, `logic_analyzer_graph_compiler`, `logic_analyzer_graph_nodes`, `node_graph`,
`logic_analyzer_viewer`, reusable widgets, and `logic_analyzer_ui` contain no target conditionals,
target-selected files, or target-specific dependencies. Portable processing code in
`logic_analyzer_processing` follows the same rule. Shared codecs, indexes, cache policy, source
preparation, graph lowering, viewer queries, and node contracts therefore compile from exactly the
same source files. Runtime capability values describe what the injected host can do.

### Browser constraints informing the proposal

- Rust's `wasm32-unknown-unknown` target supplies `core` and `alloc`, while `std::fs` operations
  fail and `std::thread::spawn` panics:
  <https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html>.
- OPFS supplies origin-private random-access storage. Its synchronous access handle is available
  only in a Web Worker and its data remains subject to browser quota and site-data deletion:
  <https://developer.mozilla.org/en-US/docs/Web/API/File_System_API/Origin_private_file_system>.
- Worker messages normally clone data; transferable `ArrayBuffer` ownership avoids copying but
  detaches the sender's buffer:
  <https://developer.mozilla.org/en-US/docs/Web/API/Web_Workers_API/Transferable_objects>.
- WebUSB is restricted to secure contexts, has limited browser availability, and is available from
  Web Workers on supporting browsers:
  <https://developer.mozilla.org/en-US/docs/Web/API/WebUSB_API>.

### Proposed invariants

- One encoded representation and one query implementation serve native and web repositories.
- Reusable core crates compile the same module tree and Rust source on native and web targets.
- `logic_analyzer_platform` is the only reusable crate with general target selection or
  target-specific dependencies.
- Native and web application crates contain bootstrap and injection only.
- Processing exceptions are complete, explicitly allowlisted file-I/O or USB adapter leaves; they
  do not include node state, builders, parsers, protocol state machines, or synthetic substitutes.
- The in-memory repository is a first-class backend, not an alternate derived-data model.
- Persistence is a repository capability, not a compiler or lane-shape difference.
- Mmap is a byte-backing optimization, not a public storage contract.
- Paths and browser handles do not cross into graph state, compiler IR, indexes, or viewers.
- Browser acquisition and permissions finish before synchronous shared processing begins.
- The UI thread does not perform unbounded parsing, encoding, indexing, or cache cleanup.
- Parallel completion order never changes published payload order or artifact bytes.
- Saved and persisted formats contain no `usize` fields.
- Viewer memory is bounded independently of capture duration on every backend that supplies a
  bounded artifact repository.
