# DSL Loader - Internal Design Documentation

This document describes the internal architecture, design decisions, and evolution of the DSL loader streaming system.

---

## Table of Contents

1. [Streaming Architecture](#streaming-architecture)
2. [Generic Channel Support](#generic-channel-support)
3. [Port-Based Connection API Design](#port-based-connection-api-design)
4. [Architecture Migration Summary](#architecture-migration-summary)

---

## Streaming Architecture

### Architecture Overview

The system uses a **thread-per-node architecture** for real-time signal processing:

- **Thread-per-node with crossbeam channels**
- All nodes implement `ProcessNode` trait
- Automatic backpressure via bounded channels
- High-throughput parallel processing
- **Optimized for**: Live hardware capture at 250MHz-1GHz, real-time processing
- **Components**: Scheduler, work nodes, streaming decoders

### Architecture Components

#### 1. Sample Types ([src/nodes/sample.rs](src/nodes/sample.rs))

- `BoolSample`: Single-channel boolean with timestamp
- Used for individual signal channels in decoders and file sources

#### 2. ProcessNode Trait ([src/runtime/work.rs](src/runtime/work.rs))

- `ProcessNode`: The unified trait for all nodes
- Methods: `name()`, `should_stop()`, `num_inputs()`, `num_outputs()`, `work()`
- Sources have 0 inputs, Sinks have 0 outputs, Processors have both

#### 3. Thread-Per-Node Scheduler ([src/runtime/scheduler.rs](src/runtime/scheduler.rs))

- Spawns dedicated thread for each node
- Continuously calls node's `work()` function
- Automatic coordination via crossbeam channels
- Backpressure: nodes block when channels full/empty
- Clean shutdown via atomic stop signal

#### 4. Pipeline Builder API ([src/runtime/ports.rs](src/runtime/ports.rs))

- Type-safe connection validation at compile time
- `add_process()` for all node types (sources, sinks, processors)
- Port-based connections: `port.connect(port, &mut pipeline)`
- Configurable buffer sizes per connection
- Automatic channel creation during build phase

#### 5. DSL File Source ([src/nodes/dsl_file.rs](src/nodes/dsl_file.rs))



- `DslFileSource`: Streams samples from DSLogic .dsl files
- Adapts file I/O to streaming architecture
- Block caching with on-demand loading
- Direct bit reading via `read_bit()` method
- Supports 1-16 channels

#### 8. Streaming Decoders ([src/nodes/decoders](src/nodes/decoders))

- `StreamingSpiDecoder`: Full SPI protocol decoder (CS, CLK, MOSI, MISO)
- `StreamingParallelDecoder`: Parallel bus decoder with strobe modes
- Work with `BoolSample` inputs
- Output structured events (`SpiTransfer`, `ParallelWord`)
- Extension traits provide named port accessors

### Data Flow

```
┌─────────────────────┐
│  DslFileSource    │ (250MHz)
│    Thread 1        │
└─────┬───┬───┬───┬────┘
      │   │   │   │ BoolSample streams (12 channels)
      │   │   │   │ (bounded channels, 10K buffers)
      │   │   │   │
      ↓   ↓   ↓   ↓
┌───┐┌───┐┌───┐
│Cnt││Cnt││Cnt│  (Sample counters, Threads 3-5)
└───┘└───┘└───┘
```

### Key Design Decisions

#### Push vs Pull: Hybrid

- Crossbeam channels provide **demand-driven** (pull-like) scheduling
- Nodes **push** data into channels (data-driven)
- Runtime manages flow control via channel fullness
- Neither pure push nor pure pull—runtime arbitrates

#### Threading: Thread-Per-Node

- True parallelism on multi-core systems
- Independent node execution
- Implicit synchronization via channel operations
- No explicit locking in node logic

#### Backpressure: Automatic

- Bounded channels block when full
- Producer nodes wait if downstream can't keep up
- Prevents memory bloat
- Graceful degradation under load

#### Buffer Sizing Strategy

- **High-bandwidth paths**: Large buffers (10K-1M samples)
  - Hardware → Splitter: absorbs burst capture
- **Low-bandwidth paths**: Small buffers (1K events)
  - Decoded events → Display/Storage
- **Configurable per connection** via `connect()` buffer_size parameter

### Design Characteristics

- **Thread-per-node**: Each processing node runs in its own thread
- **Crossbeam channels**: MPSC channels for inter-node communication with bounded buffers
- **Backpressure**: Automatic flow control via bounded channels
- **Type safety**: Rust's type system ensures compile-time correctness

### Performance Characteristics

#### Bandwidth Capacity

- **12 channels @ 250MHz**: 375MB/s raw → 47MB/s packed (u16 + u64 timestamps)
- **3 channels @ 1GHz**: 375MB/s raw → 12MB/s packed (u8 + u64 timestamps)
- **Crossbeam channels**: Lock-free for single producer/consumer, minimal overhead

#### Latency

- **Buffer latency**: 10K samples @ 250MHz = 40μs
- **Thread scheduling**: ~1-10ms depending on OS load
- **Total end-to-end**: Sub-millisecond for properly tuned buffer sizes

#### CPU Usage

- **One thread per node**: 3 nodes = 3 CPU cores minimum
- **Hardware capture**: Can peg one core at 1GHz if not bulk-transferring
- **Splitter**: Lightweight, should use <10% of one core
- **Decoders**: Variable based on protocol complexity

---

## Generic Channel Support

### Overview

All streaming components support any number of channels from 1 to 16, replacing previous hardcoded implementations.

All components dynamically accept channel count:

- **DslFileSource**: File replay with configurable channel count (1-16 channels)
- **Decoders**: Work with individual BoolSample inputs

### Validation

All components validate channel counts at runtime:

- `DslFileSource::new(file, num_channels)` returns error if file has fewer channels
- Decoders validate input port counts based on configuration

---

## Port-Based Connection API Design

### Goal

Provide an ergonomic API for connecting nodes without manual channel management:

```rust
// Desired API
splitter.d0().connect(decoder.clk(), &mut pipeline);
splitter.d1().connect(decoder.cs(), &mut pipeline);
```

### Evolution

#### Initial State: Manual Channel Management

Manual channel creation with boilerplate:

```rust
let (tx, rx) = bounded(1000);
let spi_inputs = vec![rx_clk, rx_cs, rx_mosi, rx_miso];
scheduler.start_process(decoder, spi_inputs, vec![tx_out], stop);
```

#### Final Solution: Extension Traits + Pipeline API

1. **Extension Traits for Named Ports**

Provide domain-specific port names without coupling to Pipeline:

```rust
pub trait SplitterPorts {
    fn splitter_input(&self) -> InputPort<Sample>;
    fn d0(&self) -> OutputPort<BoolSample>;
    fn d1(&self) -> OutputPort<BoolSample>;
    // ... d2-d15
}

impl SplitterPorts for ProcessHandle {
    fn d0(&self) -> OutputPort<BoolSample> {
        self.output(0)
    }
    // ...
}
```

2. **Pipeline API for Connection Management**

Pipeline tracks connections and creates channels during build:

```rust
let mut pipeline = Pipeline::new();

let source = pipeline.add_process(DslFileSource::new("scan.dsl", 16)?);
let splitter = pipeline.add_process(Splitter::new(16));

source.output().connect(splitter.input(0), &mut pipeline);

let scheduler = pipeline.build()?; // Creates channels and wires nodes
```

### Design Challenges Solved

#### Challenge 1: Scheduler Integration

**Solution**: Two-phase approach

1. **Connection phase**: Track port connections in Pipeline metadata
2. **Build phase**: Create all channels based on connection graph
3. **Start phase**: Launch nodes with their pre-wired channels

#### Challenge 2: Named vs Indexed Ports

**Solution**: Extension traits per node type

Different nodes have different port configurations:

- **Splitter**: Uses `SplitterPorts` trait → `d0()-d15()` methods
- **SPI Decoder**: Uses `SpiDecoderPorts` trait → `clk(), cs(), mosi(), miso()` methods
- **Generic nodes**: Use `input(N)` and `output(N)` for indexed access

Benefits:

- No coupling between Pipeline and specific node types
- Type-safe port names
- IDE autocomplete support
- Future-ready for GUI integration

#### Challenge 3: Type Safety

**Solution**: Generic port types with compile-time checking

```rust
pub struct OutputPort<T> {
    node_id: NodeId,
    port_index: usize,
    _phantom: PhantomData<T>,
}

pub struct InputPort<T> {
    node_id: NodeId,
    port_index: usize,
    _phantom: PhantomData<T>,
}
```

Type checking happens during `connect()`:

```rust
impl<T: Send + 'static> OutputPort<T> {
    pub fn connect(
        self,
        input: InputPort<T>,  // Must match T
        pipeline: &mut Pipeline,
    ) -> &mut Pipeline {
        pipeline.connect_ports::<T>(/* ... */);
        pipeline
    }
}
```

Incompatible types fail at compile time:

```rust
// ✅ Valid: BoolSample -> BoolSample
splitter.d0().connect(decoder.clk(), &mut pipeline);

// ❌ Compile error: BoolSample -> SpiTransfer (type mismatch)
splitter.d0().connect(printer.input(), &mut pipeline);
```

### Benefits

1. **Clean API**: Declarative port-based connections
2. **Type Safety**: Compile-time type checking where possible
3. **No Manual Channels**: Pipeline handles all channel creation
4. **Domain-Specific Names**: Extension traits provide meaningful port names
5. **Decoupled**: Pipeline doesn't know about specific node types
6. **Future-Ready**: Port names support future GUI/IDE integration

---

## Architecture Migration Summary

### Changes Made

Successfully removed the dual architecture and kept only the streaming approach.

### Files Removed

#### Old Pull-Based Architecture

- `src/nodes/node.rs` - Node trait
- `src/nodes/port.rs` - Port trait
- `src/nodes/cursor.rs` - Cursor for sequential navigation
- `src/nodes/boolean.rs` - Boolean logic nodes (AND, OR, NOT, XOR)
- `src/nodes/const_node.rs` - Constant value node
- `src/nodes/state.rs` - SpiControlledState (depended on old SpiDecoderNode)

#### Old Protocol Decoders

- `src/nodes/decoders/spi_decoder.rs` - Pull-based SPI decoder (Iterator)
- `src/nodes/decoders/parallel_decoder.rs` - Pull-based parallel decoder (Iterator)

#### Old Examples

- `examples/spi_decode.rs` - Used old Stream/Cursor API
- `examples/parallel_decode.rs` - Used old Stream/Cursor API
- `examples/node_based_decode.rs` - Used old Port/Node API with SpiControlledState

#### Old Documentation

- `DUAL_ARCHITECTURE.md` - Described dual architecture approach
- `STREAM_TRAIT.md` - Documented old Stream trait usage

### What Was Kept

#### Stream Trait Elimination

The Stream trait was initially kept for internal use but later removed entirely:

- **Previous approach**: Stream trait provided `peek()`/`advance()` interface
- **Current approach**: Direct `read_bit(channel, position)` method
- **Benefit**: Simpler, more direct access pattern with on-demand block caching

#### DslFileSource

- **File**: `src/nodes/dsl_file.rs`
- **Purpose**: Read DSLogic .dsl files from ZIP archives
- **API**: `read_bit(channel, position)` method for direct bit access
- **Features**: Block caching with Arc<Mutex<HashMap>>
- **Used by**: Streaming pipeline for file replay

### Results

#### Tests

✅ **15 tests passing**

- Sample type tests (3ch, 12ch, 16ch, masking, validation)
- Splitter tests (3ch, 12ch, 16ch)
- Decoder creation tests (SPI, Parallel)
- Runtime tests (Scheduler, Pipeline)

#### Examples

✅ **3 streaming examples**

- `pipeline_demo.rs` - Pipeline API demonstration
- `hardware_streaming.rs` - Live capture demo
- `streaming_file_decode.rs` - File replay with parallel decoder

#### Build

✅ **Clean build**

- No clippy warnings
- All dependencies resolved
- Examples compile successfully

### API Changes (Breaking)

#### Removed from Public API

- `Node` trait
- `Port<T>` type
- `Cursor` type
- `Edge` type
- `Stream` trait
- `SpiDecoderNode`, `ParallelDecoderNode` (old decoders)
- `SpiControlledState`
- Boolean nodes (AndNode, OrNode, NotNode, XorNode)
- ConstNode

#### Current Public API

```rust
// Sample types
pub use BoolSample;

// Streaming nodes
pub use DslFileSource;

// Decoders
pub use {SpiDecoder, ParallelDecoder};
pub use {SpiMode, SpiTransfer, StrobeMode, ParallelWord};

// Runtime
pub use {Pipeline, Scheduler, ProcessNode};

// Extension traits
pub use {SplitterPorts, SpiDecoderPorts};
```

### Benefits

1. **Simpler API**: Single architecture, clearer mental model
2. **Better Performance**: Thread-per-node parallelism optimized for real-time
3. **Cleaner Code**: Removed ~1500 lines of legacy code
4. **Maintainability**: Single code path, easier to test and debug
5. **Type Safety**: Compile-time port type checking
6. **Ergonomics**: Named ports via extension traits
7. **Future-Ready**: Architecture designed for live hardware integration and GUI tools
