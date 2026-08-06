# Testing Strategy

The portable test suite verifies each crate using only the crate's source,
checked-in test data, the Rust toolchain, and its declared dependencies. A
normal `cargo test -p <crate>` invocation does not require environment
variables, sibling working directories, installed device software, attached
hardware, or network access.

## Test boundaries

- A crate owns the fixtures needed to test its behavior. Fixtures are stored
  below that crate's `test_data/` directory or generated deterministically by
  the test itself.
- Tests consume another workspace crate only through that crate's supported
  public API and only when the dependency is part of the crate under test's
  declared contract. They do not reach into another crate's implementation,
  test-data directory, or test-only module.
- Reusable deterministic fakes belong to a test-only support crate. Such a
  crate exposes test models and data only; it does not own production runtime,
  UI, device, protocol, or storage behavior.
- Protocol fixtures model only the behavior required by the assertion. They
  use project-owned names and formats so upstream packages cannot silently
  change the test contract.
- The repository structure check rejects ignored Rust tests, runtime
  environment-variable access from test modules, fixtures outside the owning
  crate, required fixtures not tracked by Git, and test-only workspace
  dependencies that belong in the top-level integration package. A crate may
  use a workspace dependency in tests when it is already part of that crate's
  production contract, or may use the neutral test-support crate for shared
  deterministic data models.
- UI component tests use local graph, host, capture-export, and acquisition
  implementations. Native dialog and filesystem-export adapters are optional
  production capabilities enabled by the native application crate, not part of
  the default `logic-analyzer-ui` test dependency graph. End-to-end UI,
  compiler, and built-in-node composition belongs to the top-level integration
  package.
- Graph-runtime source-preparation tests use immediate or manually controlled task
  executors, and graph-runtime cache-pruning tests inject cache availability and
  cleanup outcomes by key. `platform_runtime` tests portable executor and worker-queue policy;
  native/browser worker and persistent-store conformance is tested by the component that owns each
  adapter.
- Compiler tests construct neutral `node_graph_document::GraphState` values and remain independent
  of the node editor and egui. Node-definition migrations and state-dependent socket schemas are
  covered at the editor/application load boundary.
- Concrete-node registration contract tests use the editor's document builder;
  only tests of editor interaction or presentation construct the graph widget.
- Capture-file parsers and replay sources consume the processing-owned
  `CaptureArchive` contract. Their unit tests inject in-memory entries; a
  focused generated-file test covers the native ZIP adapter, while generated
  archive and repository-artifact integration tests cover the complete indexed-reader path.
- Concrete file sinks create, append, write, and flush through the private
  `OutputStorage` contract. Sink tests inject in-memory output files and
  controlled create, write, and flush failures; native filesystem tests cover
  path creation, rollover, and persisted bytes.

## Continuous integration

CI runs each workspace crate as its own `cargo test -p <crate>` matrix entry.
Only after every crate passes does it run the top-level
`logic-analyzer-examples` integration package and the compile-time plugin-link
test. Architecture checks, Clippy, wasm compilation, deterministic benchmark
compilation, and manual-validation-tool compilation are separate jobs so a
failure identifies the affected boundary. External validation commands are
compiled in CI but are not executed there.

## Python decoder fixtures

Sigrok decoder tests use small project-owned Python packages. A fixture has a
unique test-only decoder ID and declares only the inputs, outputs, options, and
runtime behavior exercised by its test. Tests do not discover or execute an
installed upstream Sigrok decoder tree.

## External validation

Hardware capture and upstream-compatibility checks are manual validation, not
portable crate tests. They are kept separate from the ordinary test suite,
state their prerequisites, and do not make an unavailable local resource a
test failure. Their assertions complement rather than replace deterministic
tests using checked-in fixtures and fakes.

The native application command is the authoritative end-to-end graph runtime
benchmark because it exercises the durable repository, saved application
subscription plan, native source factories, and runtime completion boundary
used by the interactive Run command:

```console
cargo run --release --bin logic-conduit -- \
  run graphs/spi_controlled_decode.json --json > ui-equivalent-run.json
```

The command removes only the selected graph's previous derived entries, keeps
its raw waveform index, executes configured file sinks, writes progress to
standard error, and reports preparation, cache-clear, execution, total time,
capture real-time factor, final node counts, and persistent cache sizes. Run it
from an otherwise idle machine. A cold raw-index build and a warm indexed run
are distinct measurements.

The waveform-index profile builds a cold index in a fresh in-memory artifact
repository with the platform-selected work executor. Its JSON report separates
packed-block reading/decompression, handoff copying, summary-kernel work,
artifact publication, and total wall time. Per-stage worker time is cumulative,
so parallel summary work can exceed wall time. Run it from an otherwise idle
machine and retain the report beside the matching capture baseline:

```console
cargo bench -p logic-analyzer-examples --bench compiler_capture -- \
  waveform-index-profile /path/to/reference.dsl > waveform-index-profile.json
```

Use the isolated native durable repository probe to include real filesystem cache publication, or
the concurrency probe to compare cold durable builds across bounded worker counts without reading,
removing, or warming the application's cache:

```console
cargo bench -p logic-analyzer-examples --bench compiler_capture -- \
  waveform-index-persistent-profile /path/to/reference.dsl
cargo bench -p logic-analyzer-examples --bench compiler_capture -- \
  waveform-index-concurrency-profile /path/to/reference.dsl
```

Profile derived-cache generation through the checked-in graph with a prebuilt waveform index and
an isolated native durable repository:

```console
cargo bench -p logic-analyzer-examples --bench compiler_capture -- \
  derived-storage-profile /path/to/reference.dsl > derived-storage-profile.json
```

The report separates derived data-block, data-segment, final-index, and manifest
repository operations; inventories artifact counts and bytes; records host work-task time, process
CPU utilization, output identity, total pipeline wall time, and per-lane index-to-manifest final
publication latency. Repository and host-work times are cumulative and may exceed or overlap wall
time. Set `RUST_LOG=parallel_decoder_phase_profile=debug` to add one opt-in Parallel Decoder summary
covering input/dispatch, ordered-completion wait, merge/assembly, sampling publication, and output
send time without enabling per-fragment debug logging. The summary also reports output batch and
word counts, maximum pending batch size, and destination fan-out for bounded-coalescing analysis.

The compiler capture tool contains isolated graph-runtime timing probes and
full-capture differential validations. It requires an explicit,
developer-supplied DSL capture path and runs in the release benchmark profile:

```console
cargo bench -p logic-analyzer-examples --bench compiler_capture -- \
  <command> /path/to/capture.dsl
```

Run it with `--help` to list its timing and validation commands. These commands
are intentionally absent from Cargo's test harness because their input is
developer-supplied and their execution time depends on the complete capture.
`baseline` loads the checked-in `graphs/spi_controlled_decode.json` document,
overrides only its capture and output paths, and prints a versioned JSON report
to standard output. The report identifies the graph and capture, separates
graph load, lowering, pipeline startup, execution, storage inspection, and
output fingerprinting time, and records average CPU cores, peak RSS, throughput,
real-time factor, derived-lane and persistent-cache sizes, output names/sizes/
BLAKE3 hashes, and cancellation latency. Run it from an otherwise idle machine
and redirect standard output when retaining a baseline; progress and the short
summary use standard error:

```console
cargo bench -p logic-analyzer-examples --bench compiler_capture -- \
  baseline /path/to/reference.dsl > reference-pipeline.json
```

Execution and cancellation use fresh in-memory derived repositories and
separate child processes. This prevents a previous run from becoming a cache
hit and gives cancellation its own clean pipeline, but deliberately excludes
native durable-publication cost. These probes compare decoder algorithms,
protocol selection, ordering, output integrity, CPU use, and cancellation;
they are not substitutes for the UI-equivalent command above. Environment,
graph, capture-cache, and output fingerprints make their reports comparable
without checking a developer capture or generated data into the repository.

`live-viewer-runtime` attaches the production logic-analyzer viewer to the running graph and drives
it through a headless egui update at a 16 ms cadence. Every frame contains pointer input. The probe
reports snapshot-query and complete input-frame p50/p95/p99 latency, frames exceeding 8 ms and
16 ms, and per-lane snapshot distributions. It prebuilds the waveform index, then runs graph
processing and segmented derived-cache generation through an isolated native durable repository.
Its JSON report includes labeled finite and long-running host work, sampled per-node work and thread
CPU, artifact publication, final index-to-manifest latency, process resources, output identity, and
the foreground measurements. It uses the same complete capture but deliberately does not enforce
the pipeline real-time-factor threshold: foreground responsiveness and processing throughput remain
separate measurements.

```console
cargo bench -p logic-analyzer-examples --bench compiler_capture -- \
  live-viewer-runtime /path/to/reference.dsl
```

`validate-compiled` runs the compiled graph and the independent reference
pipeline, then compares the complete output manifest by canonical output name,
size, and BLAKE3 hash. It adds one explicit sparse control-lane subscription;
connected graph outputs continue to follow the compiler's normal generic
retention policy, including the indexed derived-cache path. The generated
binary files, normalized CSV manifests, and derived-data caches live under one
operating-system temporary directory outside the repository and are removed
after the command. The compiled and reference stages execute in separate,
sequential child processes so each stage has an enforceable memory-reclamation
boundary. Persistent waveform-index and raw-block artifacts remain owned by the injected
artifact repository and should also be kept outside the source repository.

```console
cargo bench -p logic-analyzer-examples --bench compiler_capture -- \
  validate-compiled /path/to/reference.dsl
```

The derived-word store throughput guard uses a deterministic generated
eight-bit workload with a small quantized timestamp-delta palette. Each append
contains several complete blocks, so the command exercises bounded parallel preparation,
out-of-order completion handling, and ordered publication rather than measuring only the codec:

```console
cargo run --release -p signal-derived --bin derived-word-store-bench
```

It is a non-test binary so ordinary and ignored test runs neither discover nor
execute the performance guard. The native store tests separately force prepared blocks to arrive
out of sequence and verify that readers still observe contiguous input order.

The file-backed parallel-decoder benchmark characterizes packed scanning at fixed worker counts
without checking a capture into the repository. Its worker sweep requests 1, 2, 4, 8, 16, and 32
workers, reports effective host-capped concurrency, CPU usage, peak RSS, normalized packed-input
throughput, and fragment/reorder memory bounds. Use `count` to require identical output at every
worker count and `discard` to isolate scan throughput from downstream word transport:

```console
cargo run --release -p logic-analyzer-examples --bin parallel-decoder-bench -- \
  /path/to/reference.dsl --mode stream --sink count --worker-sweep
```

The U3Pro16 sustained-ingest benchmark uses a generated transport while still
exercising the concrete streaming driver, capture store, growing index, viewer
queries, and a lagging consumer:

```console
cargo run --release -p logic-analyzer-examples --features developer-tools \
  --bin u3pro16-streaming-bench
```

The feature exposes only the benchmark entry point. Its generated USB
transport remains private to the U3Pro16 source module.

U3Pro16 hardware validation is an explicit command requiring a connected
device. FPGA validation also requires the exact image as a positional path:

```console
cargo run --release -p logic-analyzer-app-native \
  --features developer-tools --bin u3pro16-hardware-validation -- \
  fpga /path/to/DSLogicU3Pro16.bin
cargo run --release -p logic-analyzer-app-native \
  --features developer-tools --bin u3pro16-hardware-validation -- capture
```

Upstream Sigrok compatibility validation requires an explicit decoder tree.
The oracle command additionally requires `pkg-config`, a C compiler, and an
installed libsigrokdecode development package:

```console
cargo run --release -p logic-analyzer-app-native \
  --features developer-tools --bin sigrok-upstream-validation -- \
  chunk-boundaries /path/to/libsigrokdecode/decoders
cargo run --release -p logic-analyzer-app-native \
  --features developer-tools --bin sigrok-upstream-validation -- \
  oracle /path/to/libsigrokdecode/decoders
```

Optional `--pkg-config` and `--cc` arguments select non-default oracle tooling.
These external validations are non-test binaries and are never discovered by
ordinary or ignored Cargo test runs.
