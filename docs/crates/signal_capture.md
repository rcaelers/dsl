# `signal_capture` Design

## Responsibility

`signal_capture` owns immutable generic signal capture: packed and edge payloads, source and index
contracts, bounded query and worker protocols, random-access `EdgeQuery`, and finite artifact-backed
waveform indexes. Its crate root is the supported facade; implementation modules are private.

## Dependency boundary

The crate depends only on `platform_artifacts`, `platform_runtime`, `signal_runtime`, and portable
serialization, hashing, error, and channel libraries. It uses `platform_runtime` for finite work
and worker-operation contracts and `signal_runtime` for typed streams. It has no dependency on acquisition
sessions, derived-data stores, graph crates, concrete formats or devices, UI, or platform adapters.

Growing live indexes remain with the capture-session owner because they consume mutable session
storage. They reuse an explicit waveform-summary grid contract from this crate, so finite index
algorithms and query semantics remain shared without reversing the dependency.

## Finite waveform index

The finite index turns a generic `CaptureDataSource` into a bounded `CaptureIndex` without knowing
the source format, graph node, viewer, or physical repository implementation:

Implementation: [crates/signal_capture/src/waveform_index/](../../crates/signal_capture/src/waveform_index)

```text
CaptureDataSource
  └─ IndexBuilder       builds channel/block summary leaves
       └─ IndexWriter   publishes bounded immutable segments, then the root
            └─ IndexReader / IndexSampler
                 ├─ sampled-window summaries
                 ├─ exact transition queries
                 └─ cached packed-block reads
```

The caller supplies an artifact repository and `WorkExecutor`. Native repositories may retain
mmap-backed regions and memory repositories retain owned chunks; index code consumes only immutable
byte regions. The completed `IndexSampler` is exposed as `Box<dyn CaptureIndex>`, so consumers do
not depend on the storage implementation.

### Index terminology

| Term | Meaning |
|---|---|
| Block | The source-owned packed-capture unit described by `CaptureMetadata::samples_per_block` |
| Leaf | The serialized summary for one `(channel, block)`: valid samples, flags, and optional L1/L2/L3 bitmaps |
| Directory entry | The root record locating one leaf and duplicating its coarsest summary |
| Segment artifact | Up to 64 channel-major leaf payloads in one immutable publication |
| Root artifact | Capture metadata and the complete segment directory for one source identity |
| Raw-block artifact | A lazily published packed source block used for exact and deep-zoom queries |

Every `(channel, block)` pair has a directory entry. The duplicated L3 summary lets coarse queries
avoid loading the leaf segment.

### Summary hierarchy

Each active block records transition and last-value summaries at three levels. A transition bit
means that at least one level change occurred in its sample group; the matching last-value bit
records the signal level at the group boundary.

```text
L1  4096 × u64   one bit per      64 raw samples
L2    64 × u64   one bit per   4,096 raw samples
L3     1 × u64   one bit per 262,144 raw samples
```

For the usual `2^24`-sample block, the hierarchy occupies 66,576 bytes for an active block.
Constant blocks store only valid-sample, first-level, and last-level metadata. If a transition lies
exactly between blocks, the builder compares the predecessor's last level with the next block's
first level and adds the boundary transition to the next leaf, including when that leaf would
otherwise be constant.

### Repository format

The root format uses magic `CAPIDX07`, format version 8, a 96-byte header, and 40-byte directory
entries. The header records source revision, capture dimensions, sample rate, directory location,
and payload location. Each channel-major directory entry records its offset and length within a
segment, transition/first/last flags, and duplicated L3 transition and last-value words.

Leaves are grouped 64 at a time. `IndexWriter` publishes complete segment artifacts as the build
advances and publishes the root last, so an interrupted generation is not discoverable. On open,
`IndexReader` validates the format version, source revision, dimensions, and sample rate. A stale or
incompatible root is rebuilt. The reader retains bounded segment and decoded-leaf caches.

### Raw-block artifacts

Exact queries publish a complete packed artifact per `(source identity, channel, block)` on first
use. Reopened samplers reuse it without reading or decompressing the concrete source again.
External one-shot packed-block consumers do not populate this cache implicitly, and publication
never exposes a partial block.

### Building

`IndexBuilder` enumerates every `(channel, block)` job and uses at most 12 bounded workers, capped by
the injected executor's advertised parallelism and the job count. Each worker opens an independent
source reader and builds one summary leaf from packed samples. A bounded collector restores
channel-major order, patches boundary transitions, and streams leaves into `IndexWriter`.

Progress is reported as completed and total root jobs. Cancellation stops further submission and
does not publish the root artifact.

### Querying

`CaptureIndex::sampled_window` receives channels, a half-open sample range, and a target-point
budget. It clamps the range and selects one of two result shapes:

- Exact windows return individual `CaptureTransition` values by reading packed samples.
- Wider windows return bounded `CaptureWaveformSegment` values at block, L3, L2, or L1 resolution.

The exact threshold is at least 64 samples per target point, with a floor of 4,096 samples. For a
summary query, every rendered bucket is classified as a stable level, an exact boundary edge, or
activity with truthful first and last levels. Summary code never invents an edge position within a
bucket.

Exact transition search descends the summary hierarchy to one 64-sample L1 group before consulting
packed data. Consumers can therefore locate exact boundaries with bounded sampled-window queries
without scanning the complete capture or depending on display resolution.

## Errors and tests

`Error` and `Result` bound capture parsing, indexing, and query failures. Unit tests cover capture
worker protocols, block ownership, finite index construction, bounded sampling, persistence, and
random-access queries. Architecture tests reject session, derived-data, graph, and UI dependencies.
