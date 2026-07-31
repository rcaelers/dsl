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
- Compiler source-preparation tests use immediate or manually controlled task
  executors, and compiler cache-pruning tests inject cache availability and
  cleanup outcomes by key. Native worker and persistent-store conformance is
  tested by the component that owns each adapter.
- Compiler tests construct saved graph documents through the headless
  `node_graph::api::GraphDocumentBuilder`. Node-definition migrations and
  state-dependent socket schemas therefore remain active without constructing
  `NodeGraphWidget` or importing `egui` in the compiler crate.
- Concrete-node registration contract tests use that same headless builder;
  only tests of editor interaction or presentation construct the graph widget.
- Capture-file parsers and replay sources consume the processing-owned
  `CaptureArchive` contract. Their unit tests inject in-memory entries; a
  focused generated-file test covers the native ZIP adapter, while generated
  archive and sidecar integration tests cover the complete indexed-reader path.
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

The compiler capture tool contains the graph-runtime timing probes and the
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

Execution and cancellation use fresh derived-cache directories and separate
child processes. This prevents a previous run from turning the execution
measurement into a derived-data cache hit and gives cancellation its own clean
pipeline. Throughput comparisons use the application's normal preloaded and
indexed capture state and an otherwise idle machine; cold filesystem-cache runs
measure storage warmup as well as pipeline execution and are recorded
separately. Environment, graph, capture-cache, and output fingerprints make
reports comparable without checking a developer capture or generated data into
the repository.
`validate-compiled` runs the compiled graph and the independent reference
pipeline, then compares the complete output manifest by canonical output name,
size, and BLAKE3 hash. It adds one explicit sparse control-lane subscription;
connected graph outputs continue to follow the compiler's normal generic
retention policy, including the indexed derived-cache path. The generated
binary files, normalized CSV manifests, and derived-data caches live under one
operating-system temporary directory outside the repository and are removed
after the command. The compiled and reference stages execute in separate,
sequential child processes so each stage has an enforceable memory-reclamation
boundary. Persistent waveform-index sidecars remain owned by the
developer-supplied capture and should also be kept outside the repository.

```console
cargo bench -p logic-analyzer-examples --bench compiler_capture -- \
  validate-compiled /path/to/reference.dsl
```

The derived-word store throughput guard uses a deterministic generated
eight-bit workload with a small quantized timestamp-delta palette and exercises
the supported indexed writer boundary:

```console
cargo run --release -p signal-processing --bin derived-word-store-bench
```

It is a non-test binary so ordinary and ignored test runs neither discover nor
execute the performance guard.

The U3Pro16 sustained-ingest benchmark uses a generated transport while still
exercising the concrete streaming driver, capture store, growing index, viewer
queries, and a lagging consumer:

```console
cargo run --release -p logic-analyzer-processing \
  --features developer-tools --bin u3pro16-streaming-bench
```

The feature exposes only the benchmark entry point. Its generated USB
transport remains private to the U3Pro16 source module.

U3Pro16 hardware validation is an explicit command requiring a connected
device. FPGA validation also requires the exact image as a positional path:

```console
cargo run --release -p logic-analyzer-processing \
  --features developer-tools --bin u3pro16-hardware-validation -- \
  fpga /path/to/DSLogicU3Pro16.bin
cargo run --release -p logic-analyzer-processing \
  --features developer-tools --bin u3pro16-hardware-validation -- capture
```

Upstream Sigrok compatibility validation requires an explicit decoder tree.
The oracle command additionally requires `pkg-config`, a C compiler, and an
installed libsigrokdecode development package:

```console
cargo run --release -p logic-analyzer-processing \
  --features developer-tools --bin sigrok-upstream-validation -- \
  chunk-boundaries /path/to/libsigrokdecode/decoders
cargo run --release -p logic-analyzer-processing \
  --features developer-tools --bin sigrok-upstream-validation -- \
  oracle /path/to/libsigrokdecode/decoders
```

Optional `--pkg-config` and `--cc` arguments select non-default oracle tooling.
These external validations are non-test binaries and are never discovered by
ordinary or ignored Cargo test runs.
