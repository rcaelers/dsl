# DSL Loader - API Documentation

A high-performance streaming architecture for DSLogic logic analyzer signal processing in Rust.

---

## Table of Contents

1. [Overview](#overview)
2. [Quick Start](#quick-start)
3. [Core Components](#core-components)
4. [Pipeline API](#pipeline-api)
5. [Protocol Decoders](#protocol-decoders)
6. [Examples](#examples)
7. [Performance](#performance)
8. [Building and Testing](#building-and-testing)

---

## Overview

### Features

- **Thread-per-node parallel processing** using crossbeam channels
- **Generic channel support**: 1-16 channels with bit-packing for bandwidth reduction
- **Full protocol decoders**: SPI and parallel bus decoders
- **Hardware and file sources**: Live capture (skeleton) and DSL file replay
- **Type-safe Pipeline API**: Compile-time type checking for node connections
- **Named ports**: Domain-specific port names via extension traits

### Architecture

The system uses a thread-per-node streaming architecture:

```text
┌──────────────────────┐     ┌─────────────┐     ┌───────┐
│ DslFileSource│────▶│ SpiDecoder  │────▶│ Sink  │
│      (Thread 1)       │     │  (Thread 2) │     │(Thr 3)│
└──────────────────────┘     └─────────────┘     └───────┘
      BoolSample×N             SpiTransfer          Print
```

Each node runs in its own thread, connected by bounded crossbeam channels for automatic backpressure.

---

## Quick Start

### File Replay Example

```rust
use dsl::{Pipeline, DslFileSource, BoolSample};

// Create pipeline
let mut pipeline = Pipeline::new();

// Add file source (reads file and outputs 16 individual channel streams)
pipeline.add_process(
    "source",
    DslFileSource::new("capture.dsl", 16)?,
)?;

// Connect individual channels to sinks (example: first 4 channels)
for i in 0..4 {
    pipeline.add_process(
        &format!("counter_{}", i),
        SampleCounter::new(i, 1000),
    )?;
    pipeline.connect("source", &format!("d{}", i), &format!("counter_{}", i), "input")?;
}

// Build and run
let scheduler = pipeline.build()?;
scheduler.wait();
```

See [examples/spi_controlled_decode.rs](examples/spi_controlled_decode.rs) for complete example.

### Hardware Capture Example

```rust
use dsl_loader::{Pipeline, HardwareSource, CaptureConfig};

// Configure capture (12 channels @ 250MHz)
let config = CaptureConfig::new(
    250_000_000,  // 250MHz sample rate
    12,           // 12 channels
    65536,        // Buffer size
);

// Create pipeline
let mut pipeline = Pipeline::new();

// Add hardware source
pipeline.add_process("source", HardwareSource::new(config))?;

// Add decoder or sink
pipeline.add_process("decoder", MyDecoder::new())?;

// Connect nodes
pipeline.connect("source", "raw_data", "decoder", "input")?;

// Build and run
let scheduler = pipeline.build()?;
scheduler.wait();
```

See [examples/hardware_streaming.rs](examples/hardware_streaming.rs) for complete example.

---

## Core Components

### Sample Types

#### BoolSample

Single boolean channel with timestamp

```rust
use dsl_loader::BoolSample;

let sample = BoolSample::new(true, 1000);
assert_eq!(sample.value, true);
assert_eq!(sample.timestamp, 1000);
```

### Nodes

#### Sources

- **`DslFileSource`**: Reads DSLogic .dsl files and directly outputs individual channel streams (0 inputs, N outputs)  

```rust
// File source - reads and outputs 8 channels directly
pipeline.add_process(
    "source",
    DslFileSource::new("capture.dsl", 8)?,
)?;
```

#### Processing Nodes

Transform data (have inputs and outputs):

- **`SpiCommandController`**: Watches SPI transfers and emits state changes based on command values

```rust
// Enable on 0x600081, disable on 0x600000
pipeline.add_process(
    "controller",
    SpiCommandController::new(0x600081, 0x600000),
)?;
```

#### Sinks

Consume data (no outputs). Create custom sinks by implementing `SinkNode<T>`:

```rust
use dsl_loader::runtime::{ProcessNode, WorkResult, WorkError, InputChannels, OutputChannels};
use dsl_loader::nodes::BoolSample;

struct SampleCounter {
    channel_id: usize,
    count: usize,
    max_samples: usize,
}

impl ProcessNode for SampleCounter {
    fn name(&self) -> &str {
        "sample_counter"
    }

    fn num_inputs(&self) -> usize {
        1
    }

    fn num_outputs(&self) -> usize {
        0  // Sink
    }

    fn work(&mut self, inputs: &InputChannels, _outputs: &OutputChannels) -> WorkResult<usize> {
        let input = inputs.get::<BoolSample>(0)
            .ok_or_else(|| WorkError::NodeError("Missing input".to_string()))?;

        let sample = input.recv()?;
        self.count += 1;
        println!("[Channel {}] Sample {}: t={}, value={}",
            self.channel_id, self.count, sample.timestamp, sample.value);

        if self.count >= self.max_samples {
            return Ok(0);  // Stop processing
        }
        Ok(1)
    }
}
```

### Runtime

#### Pipeline

High-level API for building streaming graphs:

```rust
let mut pipeline = Pipeline::new();

// Add nodes by name
pipeline.add_process("source", my_source)?;
pipeline.add_process("process", my_process)?;
pipeline.add_process("sink", my_sink)?;

// Connect using node names and port names
pipeline.connect("source", "output", "process", "input")?;
pipeline.connect("process", "output", "sink", "input")?;

// Build and run
let scheduler = pipeline.build()?;
scheduler.wait();
```

**Features:**

- Automatic channel creation
- Type-safe connections (validated at connection time)
- Named ports for clarity
- Configurable buffer sizes

#### Scheduler

Low-level API for manual control:

```rust
use dsl_loader::Scheduler;

let mut scheduler = Scheduler::new();

// Manually wire channels and start nodes
scheduler.start_source(source, output_channel);
scheduler.start_process(process, input_channels, output_channels);
scheduler.start_sink(sink, input_channel);

// Wait for completion
scheduler.wait();
```

---

## Pipeline API

### Overview

The Pipeline API provides an ergonomic, port-based way to build streaming processing graphs without managing channels manually.

### Key Features

#### 1. No Manual Channel Management

**Old way:**

```rust
let (tx, rx) = bounded(1000);
scheduler.start_source(source, tx);
scheduler.start_sink(sink, rx);
```

**New way:**

```rust
pipeline.connect("source", "output", "sink", "input")?;
```

#### 2. Type-Safe Connections

Connections are type-checked at connection time:

```rust
// ✅ Valid: BoolSample -> BoolSample
pipeline.connect("source", "d0", "decoder", "clk")?;

// ❌ Runtime error caught at connection time: type mismatch
pipeline.connect("source", "d0", "spi_printer", "spi_input")?;  // BoolSample -> SpiTransfer
```

#### 3. Named Ports

All ports have descriptive names for clarity:

```rust
// DslFileSource ports: "d0", "d1", ... "d15" (data channels)
pipeline.connect("source", "d7", "decoder", "clk")?;

// SPI decoder ports: "clk", "cs", "mosi", "miso" (inputs), "spi_transfers" (output)
pipeline.connect("source", "d8", "spi", "cs")?;

// Custom node ports: defined in node's input_schema() and output_schema()
```

#### 4. Name-Based Node References

- **No handles returned**: `add_process()` returns `Result<(), String>` 
- **Reference by name**: Use node names in all connection calls
- **Introspection available**: `pipeline.list_node_inputs("node_name")` for port discovery

### Complete SPI Decoding Example

```rust
use dsl_loader::{
    Pipeline, DslFileSource,
    StreamingSpiDecoder, decoders::SpiMode
};

// Create pipeline
let mut pipeline = Pipeline::new();

// Add file source
pipeline.add_process("source", DslFileSource::new("capture.dsl", 16)?)?;

// Add SPI decoder
pipeline.add_process(
    "spi",
    SpiDecoder::new(SpiMode::Mode0, 8, true, false),
)?;

// Wire SPI channels (example: channels 0-3 = CLK, CS, MOSI, MISO)
pipeline.connect("source", "d0", "spi", "clk")?;
pipeline.connect("source", "d1", "spi", "cs")?;
pipeline.connect("source", "d2", "spi", "mosi")?;
pipeline.connect("source", "d3", "spi", "miso")?;

// Add sink for SPI transfers
pipeline.add_process("printer", SpiTransferPrinter::new())?;
pipeline.connect("spi", "spi_transfers", "printer", "spi_input")?;

// Build and run
let scheduler = pipeline.build()?;
scheduler.wait();
```

See [examples/pipeline_demo.rs](examples/pipeline_demo.rs) for complete example.

### Advanced Usage

#### Custom Buffer Sizes

```rust
pipeline.connect_with_buffer(
    "source",
    "d0",
    "decoder",
    "input",
    5_000_000  // Larger buffer for high-throughput
)?;
```

#### Multiple Connections

```rust
// Connect multiple channels from a source
for i in 0..4 {
    pipeline.connect(
        "source",
        &format!("d{}", i),
        "decoder",
        &format!("input_{}", i),
    )?;
}
```

---

## Protocol Decoders

### SPI Decoder

Full SPI protocol decoder with support for all modes and optional MOSI/MISO.

```rust
use dsl_loader::{SpiDecoder, decoders::{SpiMode, SpiTransfer}};

pipeline.add_process(
    "spi",
    SpiDecoder::new(
        SpiMode::Mode0,  // CPOL=0, CPHA=0
        8,               // bits per word
        true,            // has MOSI
        false,           // no MISO
    ),
)?;
```

**Supported SPI Modes:**

- Mode 0: CPOL=0, CPHA=0 (sample on rising edge)
- Mode 1: CPOL=0, CPHA=1 (sample on falling edge)
- Mode 2: CPOL=1, CPHA=0 (sample on falling edge)
- Mode 3: CPOL=1, CPHA=1 (sample on rising edge)

**Output:**

```rust
pub struct TimingInfo {
    pub timestamp_us: f64,    // Timestamp in microseconds
    pub position: u64,        // Position in capture
}

pub struct SpiTransfer {
    pub mosi: u32,
    pub miso: u32,
    pub timing: TimingInfo,
}
```

**Port Names:**

- `"clk"` - Clock input
- `"cs"` - Chip select input
- `"mosi"` - Master Out Slave In input
- `"miso"` - Master In Slave Out input
- `"spi_transfers"` - SpiTransfer output

### SPI Command Controller

Controls boolean state based on SPI command values. Useful for enabling/disabling downstream nodes based on SPI control commands.

```rust
use dsl_loader::{SpiCommandController, Pipeline, SpiDecoder, SpiMode};

let mut pipeline = Pipeline::new();

// Add SPI decoder
pipeline.add_process(
    "spi",
    SpiDecoder::new(SpiMode::Mode0, 24, true, false),
)?;

// Add command controller that watches for enable/disable commands
pipeline.add_process(
    "controller",
    SpiCommandController::new(0x600081, 0x600000), // enable, disable commands
)?;

// Connect SPI decoder to controller
pipeline.connect("spi", "spi_transfers", "controller", "spi_in")?;

// Controller emits BoolSample(true) when 0x600081 is seen
// Controller emits BoolSample(false) when 0x600000 is seen
// The boolean output can control other nodes (e.g., enable a parallel decoder)
pipeline.connect("controller", "enable_signal", "parallel", "enable")?;
```

**Features:**

- Input: `SpiTransfer` events from SPI decoder
- Output: `BoolSample` state changes (only emitted when state changes)
- Configurable enable/disable command values
- Initial state is `false`
- Only emits output when state transitions occur

See [examples/spi_controlled_decode.rs](examples/spi_controlled_decode.rs) for complete example combining SPI control with parallel bus decoding.

### Parallel Bus Decoder

Parallel bus decoder with configurable strobe modes and data width.

```rust
use dsl_loader::{ParallelDecoder, decoders::{StrobeMode, ParallelWord}};

pipeline.add_process(
    "parallel",
    ParallelDecoder::new(
        8,                      // 8 data bits
        StrobeMode::RisingEdge, // strobe trigger
        false,                  // no enable signal
    ),
)?;
```

**Strobe Modes:**

- `RisingEdge`: Capture on rising edge
- `FallingEdge`: Capture on falling edge
- `AnyEdge`: Capture on both edges
- `Level`: Capture when high

**Output:**

```rust
pub struct TimingInfo {
    pub timestamp_us: f64,    // Timestamp in microseconds
    pub position: u64,        // Position in capture
}

pub struct ParallelWord {
    pub value: u64,
    pub timing: TimingInfo,
}
```

**Features:**

- 1-64 data bits
- Optional enable signal
- Multiple strobe modes
- Configurable data width

---

## Examples

### Running Examples

```bash
# File replay with pipeline API
cargo run --example pipeline_demo

# File replay with parallel decoder
cargo run --example streaming_file_decode -- --file scan.dsl

# SPI-controlled parallel decoding
cargo run --release --example spi_controlled_decode -- \
    --file scan.dsl \
    --spi-cs 8 --spi-clk 7 --spi-mosi 6 \
    --parallel-strobe 10 --parallel-data 0 1 2 3 4 5 6 7 \
    --enable-cmd 0x600081 --disable-cmd 0x600000 \
    -n 100

# Hardware capture demo (skeleton)
cargo run --example hardware_streaming
```

### Example: Custom Sink Node

Create a custom sink to process decoded data:

```rust
use dsl_loader::runtime::{ProcessNode, WorkResult, WorkError, InputChannels, OutputChannels};
use dsl_loader::decoders::SpiTransfer;

struct SpiPrinter {
    transfer_count: usize,
}

impl SpiPrinter {
    fn new() -> Self {
        Self { transfer_count: 0 }
    }
}

impl ProcessNode for SpiPrinter {
    fn name(&self) -> &str {
        "spi_printer"
    }

    fn num_inputs(&self) -> usize {
        1
    }

    fn num_outputs(&self) -> usize {
        0  // Sink
    }

    fn work(&mut self, inputs: &InputChannels, _outputs: &OutputChannels) -> WorkResult<usize> {
        let input = inputs.get::<SpiTransfer>(0)
            .ok_or_else(|| WorkError::NodeError("Missing input".to_string()))?;

        let transfer = input.recv()?;
        self.transfer_count += 1;

        println!("SPI Transfer #{}", self.transfer_count);
        println!("  MOSI: 0x{:06X}", transfer.mosi);
        println!("  MISO: 0x{:06X}", transfer.miso);
        println!("  Time: {}", transfer.timestamp);

        Ok(1)
    }
}
```

---

## Performance

### Bandwidth

- **Bit-packing**: Up to 8x bandwidth reduction
- **12 channels @ 250MHz**: 375MB/s raw → 47MB/s packed
- **3 channels @ 1GHz**: 375MB/s raw → 12MB/s packed

### Parallelism

- **Thread-per-node**: True parallel processing on multi-core systems
- **Lock-free channels**: Crossbeam MPSC channels for minimal overhead
- **Automatic backpressure**: Bounded channels prevent memory bloat

### Latency

- **Buffer latency**: 10K samples @ 250MHz = 40μs
- **Thread scheduling**: ~1-10ms depending on OS load
- **End-to-end**: Sub-millisecond for properly tuned buffer sizes

### Buffer Sizing Strategy

- **High-bandwidth paths**: Large buffers (10K-1M samples)
  - Hardware → Splitter: absorbs burst capture
- **Low-bandwidth paths**: Small buffers (1K events)
  - Decoded events → Display/Storage
- **Configurable per connection**

---

## Building and Testing

### Build

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Build examples
cargo build --examples
```

### Testing

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_sample_channel

# Run with output
cargo test -- --nocapture
```

**Test Coverage:**

- 15 tests passing
- Sample type tests (3ch, 12ch, 16ch, masking, validation)
- Splitter tests (3ch, 12ch, 16ch)
- Decoder creation tests
- Runtime tests

### Dependencies

- `crossbeam-channel` (0.5): Lock-free MPSC channels
- `zip`: DSL file reading
- `thiserror`: Error handling
- `clap`: CLI parsing (examples only)

---

## API Comparison

### Old Manual API vs Pipeline API

| Task           | Old API                             | Pipeline API                                                       |
| -------------- | ----------------------------------- | ------------------------------------------------------------------ |
| Create channel | `bounded(1000)`                     | Automatic                                                          |
| Add nodes      | Manual wiring                       | `pipeline.add_process("name", node)?`                              |
| Wire nodes     | Pass `Vec<Sender>`, `Vec<Receiver>` | `pipeline.connect("from", "port", "to", "port")?`                 |
| Start nodes    | `scheduler.start_*(...)`            | `pipeline.build()?`                                                |
| Buffer size    | `bounded(SIZE)`                     | `pipeline.connect_with_buffer("from", "p", "to", "p", SIZE)?`    |
| Port names     | Manual indexing                     | Named ports (e.g., "d0", "clk", "spi_transfers")                  |

### Migration from Handle-Based API

If migrating from the older handle-based API:

1. Change `let handle = pipeline.add_process(node)` to `pipeline.add_process("name", node)?`
2. Replace handle-based connections like `handle.output("port").connect(other.input("port"), &mut pipeline)?` 
   with `pipeline.connect("node_name", "port", "other_name", "port")?`
3. Use `pipeline.list_node_inputs("name")` and `pipeline.list_node_outputs("name")` for introspection

The low-level Scheduler API still works for cases requiring fine control.

---

## License

[Your License Here]
