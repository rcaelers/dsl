# `logic_analyzer_capture_formats` Design

## Responsibility

`logic_analyzer_capture_formats` owns portable DSL and Sigrok archive parsing, prepared-source
metadata discovery, capture indexing adapters, and finite replay sources. Hosts supply an opaque
`PreparedByteSource`; file selection, native paths, browser handles, graph definitions, and viewer
policy remain outside the crate.

The public `dsl_file` and `sigrok_file` facades expose configuration and source-factory contracts.
Archive and random-access reader implementations remain private owner details.

## Prepared-source boundary

Both formats open independent random-access readers from a prepared source. The shared private
`CaptureArchive` contract exposes named archive entries without leaking ZIP readers or host files
into the public API. `ZipCaptureArchive` implements that contract over portable byte regions, while
parser tests use in-memory archive implementations.

Source factories discover metadata without eagerly constructing a processing node. When graph
source preparation requests an index, the format owner supplies a `CaptureDataSource` and reader
implementing the generic `signal_capture` contracts. Finite waveform indexing, its artifact format,
and sampled-window semantics remain owned by `signal_capture`.

## DSL archive layout

A `.dsl` capture is a ZIP archive with these entries:

| Entry | Description |
|---|---|
| `header` | UTF-8 metadata containing probe count and names, sample rate, total samples, total blocks, and optional trigger sample |
| `L-{channel}/{block}` | Packed logic samples for one channel and block |

The reader derives `samples_per_block` from the first logic entry's uncompressed size. Each logic
entry contains the packed bits for its `(channel, block)` coordinate. `DslCaptureReader` keeps only
a bounded decompressed-block cache and implements `BlockCaptureSource`; repository-backed raw-block
artifacts in `signal_capture` provide reuse across reopened finite indexes.

The owner validates missing entries and malformed UTF-8 or metadata before exposing them through
generic capture contracts. The compatibility path constructor is an explicitly allowlisted native
file-I/O leaf; normal application composition injects a prepared source.

## Sigrok archive boundary

The Sigrok reader owns session-metadata interpretation and supported logic-entry layouts. It
normalizes supported v1 and v2 digital sessions into the same generic capture metadata and packed
block contracts used by DSL input. Analog channels and unsupported archive versions are rejected
explicitly rather than approximated in the generic capture or viewer layers.

## Source locations

- DSL facade and source behavior:
  [crates/logic_analyzer_capture_formats/src/dsl_file/](../../crates/logic_analyzer_capture_formats/src/dsl_file)
- DSL archive reader:
  [support/dsl_file/reader.rs](../../crates/logic_analyzer_capture_formats/src/support/dsl_file/reader.rs)
- Sigrok archive reader:
  [support/sigrok_file/reader.rs](../../crates/logic_analyzer_capture_formats/src/support/sigrok_file/reader.rs)
- Shared archive adapter:
  [support/capture_archive/archive.rs](../../crates/logic_analyzer_capture_formats/src/support/capture_archive/archive.rs)

## Errors and tests

Archive and parsing failures are reported through `signal_capture::Error` with format-specific
context at the source boundary. Tests cover in-memory archives, malformed and unsupported layouts,
prepared-source construction, bounded block reuse, cooperative replay, and native path
compatibility.
