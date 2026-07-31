# Indexed Derived Word Store Design

The indexed derived-word store keeps decoded word lanes queryable without retaining every
annotation in viewer-owned memory. It provides exact values and cursor boundaries for narrow
time windows, presence summaries for overview rendering, and bounded decoded-block caching.

Primary code locations:

- `crates/signal_processing/src/derived_word_store/`;
- `crates/signal_processing/src/derived_data_collector/`;
- `crates/widgets/logic_analyzer_viewer/src/draw/derived.rs`;
- `crates/widgets/logic_analyzer_viewer/src/cursor.rs`;
- `crates/widgets/logic_analyzer_viewer/src/channel.rs`;
- `crates/logic_analyzer_graph_nodes/src/nodes/sinks/viewer/builder.rs`.

Related documents:

- [LOGIC_ANALYZER_VIEWER_DESIGN.md](LOGIC_ANALYZER_VIEWER_DESIGN.md);
- [PIPELINE_DESIGN.md](PIPELINE_DESIGN.md);
- [WASM_STORAGE_PLATFORM_DESIGN.md](WASM_STORAGE_PLATFORM_DESIGN.md).

## Responsibilities

The store:

- preserves each decoded `Word` numeric value or arbitrary-width payload, timestamp, and explicit duration;
- keeps viewer memory independent of recording duration on native builds;
- answers exact-window, presence-window, and nearest-boundary queries;
- supports queries while decoding is active;
- detects malformed blocks and stale or incomplete persistent caches;
- isolates storage failure from other consumers of the decoded word stream.

The store belongs to a derived-data collector rather than to a decoder or presentation subscriber.
Any decoder or plugin that emits retained words can use the same storage path, while an output
connected only to a non-collecting sink does not create a derived-word cache. `ParallelDecoder`
and other producers remain responsible for producing ordered word batches;
`DerivedDataCollector` materializes those batches for later subscribers.

## Architecture

```text
word-producing runtime node
  |
  | ordered Word batches
  +------------------------------> other word consumers
  |
  +----> DerivedDataCollector word lane
           |
           v
      IndexedAnnotationWriter
           |
           v
      IndexedAnnotationStore
        |       |        |
        |       |        +-- presence index
        |       +----------- committed block directory
        +------------------- bounded decoded-block cache (native)
           |
           v
      Arc<dyn AnnotationQuery>
        |                 |
        v                 v
      renderer       cursor snapping
```

The pipeline node appends blocks outside the egui thread. The viewer holds an
`Arc<dyn AnnotationQuery>` and performs bounded queries after releasing the derived-lane lock.
Only fully committed blocks are visible to readers.

## Platform model

`IndexedAnnotationStore`, `IndexedAnnotationWriter`, `AnnotationQuery`, configuration, status,
and viewer lane types exist on native and wasm.

- Native storage writes compact blocks to a temporary file. A completed store is mmap-backed and
  can be published to or reopened from the persistent cache.
- Wasm storage uses an in-memory ordered store with the same append and query semantics.

Platform selection is contained in `derived_word_store/platform/`. Generic viewer, sink, and
compiler code do not change lane shape by target. See
[WASM_STORAGE_PLATFORM_DESIGN.md](WASM_STORAGE_PLATFORM_DESIGN.md).

## Data model

The input is the runtime `Word` type:

```rust
pub struct Word {
    pub value: u64,
    pub payload: Option<WordPayload>,
    pub timestamp_ns: u64,
    pub duration_ns: u64,
}

pub enum WordPayload {
    Bytes(Arc<[u8]>),
    Text(Arc<str>),
}
```

Words arrive in nondecreasing timestamp order. Equal timestamps retain arrival order. An
out-of-order timestamp is a store error. A non-zero duration is authoritative and round-trips
exactly. Numeric words use `value` without allocating. Byte words retain their complete value at
any width, while text supplies an explicit decoder-owned label; `value` remains available as a
generic numeric tag for styling and filtering. Instantaneous words use adjacent word starts or a
cadence-bounded inferred end for display and boundary queries, so long inactive intervals remain
empty.

The public query surface is viewer-oriented and independent of the storage format:

```rust
pub trait AnnotationQuery: Send + Sync {
    fn metadata(&self) -> AnnotationStoreMetadata;

    fn presence_window(
        &self,
        start_ns: u64,
        end_ns: u64,
        target_buckets: usize,
    ) -> AnnotationQueryResult<Vec<WordPresenceBucket>>;

    fn exact_window(
        &self,
        start_ns: u64,
        end_ns: u64,
        max_words: usize,
    ) -> AnnotationQueryResult<ExactAnnotationWindow>;

    fn nearest_boundary(
        &self,
        timestamp_ns: u64,
        max_distance_ns: u64,
    ) -> AnnotationQueryResult<Option<u64>>;
}
```

An incomplete exact window causes the renderer to use the presence path; it is never drawn as if
it were a complete result. Store generations are part of the viewer sampling key so live windows
refresh when committed data changes.

## Block encoding

Native stores use append-only, versioned blocks. The default block configuration is centralized
in `BlockCodecConfig`:

| Setting | Default |
| --- | ---: |
| Maximum words | 32,768 |
| Restart interval | 512 words |
| Maximum encoded payload | 1 MiB |
| Maximum inter-word gap | 1 ms |
| Maximum timestamp span | unlimited |

A block closes when a configured count, payload, gap, or timestamp-span limit is reached, or when
the lane finishes. Gap-based closing prevents a block summary from implying activity across a
long idle interval.

Each block contains:

- restart-local timestamp groups. Dense numeric groups select the smallest exact representation
  among one constant cadence, a palette of at most 16 VLQ deltas with bit-packed palette indices,
  and unsigned per-record VLQ deltas. Legacy blocks with one VLQ delta per record remain readable;
- fixed-width values using the smallest of one, two, four, or eight bytes for that block;
- sparse duration exceptions for words with non-zero duration;
- a sparse typed payload table for arbitrary-width bytes and UTF-8 text;
- restart entries for bounded seeks within the variable-length record stream;
- a CRC32C checksum.

The file format is little-endian and versioned. Readers reject invalid magic, unsupported
versions, invalid reserved fields, overlong VLQ values, arithmetic overflow, truncated data, and
checksum mismatches.

## Presence index and exact queries

The presence index summarizes occupied time ranges and word counts at multiple resolutions.
Overview rendering requests no more buckets than the viewport needs and never invents exact
values or boundaries. Narrow views request exact annotations from the blocks intersecting the
time window.

Overview buckets use exact incremental integer partitioning, so their boundaries match the
corresponding scaled time intervals without performing wide division for every bucket. The first
leaf for a bucket is located through the 64-way summary level and then within a bounded leaf-group
and active-tail slice; partial-record counts use native-width ceiling division unless the
multiplication overflows.
Consequently a complete-capture overview touches a small bounded region of the presence index per
pixel bucket instead of repeatedly searching the complete leaf array.

Exact queries use the sorted block directory to find candidate blocks and restart entries to seek
within them. Native decoded blocks are shared through a memory-budgeted LRU keyed by store
identity and block sequence. Presence-only rendering does not populate that cache.

`nearest_boundary` considers word starts, explicit ends, and cadence-bounded inferred ends. It
checks neighboring blocks so snapping works at block boundaries and in older regions of a lane.

## Live publication

`DerivedDataCollector` owns one writer for each indexed word lane. Batch-aware inputs transfer
producer-created vectors into the collector without first flattening them into scalar channel
items. A word lane drains at most 131,072 words per scheduler call. On native hosts, its writer
turns that bounded group of producer batches into independently prepared complete blocks.
Appending:

1. validates ordering;
2. adds words to the active block builder;
3. dispatches complete builders to the shared worker pool, where each task encodes one block and
   builds its bounded presence summaries;
4. accepts prepared blocks in any completion order while retaining them by sequence number;
5. writes every contiguous prepared block through the sole file owner;
6. publishes directory and presence metadata in sequence order;
7. increments the store generation and requests a repaint.

Each writer admits one preparation task on one- and two-worker hosts and between two and four on
larger hosts. Both the in-flight task count and out-of-order completion map share this bound, and
the collector drain is bounded independently. Reaching either limit applies real backpressure.
An append call waits until all complete blocks from that call are visible, so live queries retain
the same publication contract while preparation within the call can use several cores.

The active block is exposed through an immutable hot-tail snapshot. Publication is bounded by
`LiveStoreConfig` and defaults to 262,144 words or 50 ms. Dense lanes normally commit a complete
block before either hot-tail threshold, avoiding repeated copies of an ever-growing active block.
File writes, VLQ encoding, mmap page faults, and block decoding never occur while the
published-lane catalog is locked.

Finishing closes the active block and marks the store complete. Cancelling discards unfinished
temporary state. Storage errors put the affected lane into an error state without changing the
word stream received by other graph branches.

## Persistent cache

A native persistent cache entry contains:

```text
words.dwd     encoded word blocks
words.dwi     block directory and presence index
manifest.dwm  cache identity, sizes, word count, and commit marker
```

The manifest is published last. A cache is discoverable only when its manifest, cache key, data
size, index size, directory, counts, and checksums validate. Completed data is immutable and
mmap-backed.

The compiler derives the cache key from source identity and the relevant graph configuration.
When a graph document is opened, valid entries are published as a passive derived-data preview
without executing producers or sinks. An explicit Run clears the selected graph entries before
execution and rebuilds them from the source, so Run never silently substitutes old results for
processing or sink side effects. Clearing or rejecting an entry never changes the source capture.
Native cache administration supports per-entry clearing and an LRU size budget.
Read-only inspection validates one entry without changing its LRU access time or deleting invalid
data. The Memory panel uses this contract to report data bytes, index bytes, blocks, and word counts
for the persistent entries selected by the current graph.

## Viewer integration

Word payload adapters publish an opaque `CollectedLaneQuery` backed by either
an in-memory word store or `IndexedAnnotationLane`. The indexed lane exposes
query, metadata, status, and platform-neutral store handles to its adapter.

Every collected-lane query also publishes a presentation-neutral storage snapshot. Built-in
adapters report their backing (memory, indexed working storage, or reopened persistent cache),
retained item count, resident and stored bytes, and summary/index records. Plugin adapters that do
not provide detailed accounting remain visible as adapter-managed storage.

Rendering and cursor code follow the same locking rule:

1. discover and clone the published opaque lane handle;
2. perform bounded snapshot, table, storage, or cursor-boundary queries;
3. render or select a cursor boundary without accessing adapter storage.

Every adapter may publish a snapshot generation that changes whenever visible data changes. The
viewer caches at most two sampled windows per query identity, keyed by store generation, visible
time range, viewport width, and query mode. View or query replacement invalidates the matching
entry immediately.
While a lane is live, a newer generation refreshes at most once per 50 ms; a completed lane stays
entirely on its last immutable snapshot until the view changes. Adapters without a generation
contract are queried on every use, so caching cannot make third-party data stale. Exact mode uses
the ordinary annotation-box renderer; presence mode renders summarized activity.

## Correctness invariants

1. Directory entries describe complete, checksum-valid blocks only.
2. Block sequence numbers are contiguous within a store generation.
3. Word timestamps are globally nondecreasing.
4. Concatenating decoded blocks reproduces input order and values exactly.
5. Explicit durations round-trip exactly.
6. Presence counts match the committed words represented by the index.
7. Presence queries do not report an empty bucket that contains a word.
8. Exact queries return every intersecting word or mark the result incomplete.
9. Persistent manifests refer only to synchronized data and index files with the same cache key.
10. Storage failure cannot alter another consumer's word stream.
11. Derived-lane locks are never held across storage I/O or block decoding.
12. At most the configured adaptive number of complete blocks are preparing or awaiting ordered
    publication for one writer.
13. Snapshot caching never crosses a query replacement, viewport request, or completed generation
    change; live-generation coalescing is bounded by the presentation refresh interval.

## Validation

Native and wasm contract tests cover append, exact windows, presence windows, nearest-boundary
queries, finish, cancellation, and metadata semantics. Native tests additionally cover codec
round trips, corrupt and truncated data, persistent publication and reopening, cache invalidation,
decoded-block caching, live queries, cursor behavior across blocks, deliberately reordered block
completion, and visibility at a batched append boundary.

Large-capture performance and operational follow-ups are tracked in [TODO.md](../TODO.md).

The compiler-capture `live-viewer-runtime` command runs the production viewer in a headless egui
update loop while the complete reference graph decodes. Pointer input is supplied on every frame,
and the command reports per-lane snapshot latency, input-frame p50/p95/p99, and counts above the
8 ms and 16 ms frame budgets. This foreground probe is separate from pipeline-throughput
acceptance.
