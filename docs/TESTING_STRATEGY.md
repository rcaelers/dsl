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
full-capture differential validations. It requires an explicit DSL capture
path and runs in the release benchmark profile:

```console
cargo bench -p logic-analyzer-examples --bench compiler_capture -- \
  <command> /path/to/capture.dsl
```

Run it with `--help` to list its timing and validation commands. These commands
are intentionally absent from Cargo's test harness because their input is
developer-supplied and their execution time depends on the complete capture.

The derived-word store throughput guard uses a deterministic generated
workload and exercises the supported indexed writer boundary:

```console
cargo run --release -p signal-processing --bin derived-word-store-bench
```

It is a non-test binary so ordinary and ignored test runs neither discover nor
execute the performance guard.
