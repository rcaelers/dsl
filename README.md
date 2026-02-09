# DSL Logic Analyzer Streaming Decoder

A high-performance streaming architecture for DSLogic logic analyzer signal processing in Rust.

## Features

- **Thread-per-node parallel processing** using crossbeam channels
- **Generic channel support**: 1-16 channels with bit-packing for bandwidth reduction
- **Full protocol decoders**: SPI and parallel bus decoders
- **Hardware and file sources**: Live capture (skeleton) and DSL file replay
- **Type-safe graph builder**: Compile-time type checking for node connections

## Architecture

The system uses a thread-per-node streaming architecture:

```text
┌──────────────────────┐     ┌─────────────┐     ┌───────┐
│ DslFileSource│────▶│ SpiDecoder  │────▶│ Sink  │
│      (Thread 1)       │     │  (Thread 2) │     │(Thr 3)│
└──────────────────────┘     └─────────────┘     └───────┘
      BoolSample×N             SpiTransfer          Print
```

Each node runs in its own thread, connected by bounded crossbeam channels for automatic backpressure.

## Quick Start

### File Replay Example

```rust
use dsl::{Pipeline, DslFileSource};

// Create pipeline
let mut pipeline = Pipeline::new();

// Add file source (outputs 12 individual channel streams)
let source = pipeline.add_process(
    DslFileSource::new("capture.dsl", 12)?,
);

// Connect channels directly to decoders or sinks
// Example: connect channel 7 to SPI clock
source.d7().connect(spi_decoder.clk(), &mut pipeline);

// Build and run
let scheduler = pipeline.build()?;
scheduler.wait();
```

See [examples/spi_controlled_decode.rs](examples/spi_controlled_decode.rs) for complete example.

## Core Components

### Sample Types

- **`BoolSample`**: Single boolean channel with timestamp
- Efficient representation for individual signal channels

### Nodes

- **File Sources**: `DslFileSource` (reads .dsl files and outputs individual channels)
- **Processing**: `StreamingSpiDecoder`, `StreamingParallelDecoder`, `SpiCommandController`
- **Sinks**: Custom sink nodes for analysis/output

### Runtime

- **`Pipeline`**: Type-safe graph construction with automatic node port detection
- **`Scheduler`**: Thread-per-node execution manager
- **Work Traits**: `ProcessNode` for all node types

## Protocol Decoders

### SPI Decoder

```rust
use dsl_loader::StreamingSpiDecoder;
use dsl_loader::decoders::SpiMode;

let spi = StreamingSpiDecoder::new(
    SpiMode::Mode0,
    8,      // bits per word
    true,   // has MOSI
    false,  // no MISO
);
```

Supports:

- All SPI modes (0-3)
- Configurable word size
- MOSI/MISO optional
- Outputs `SpiTransfer` events

### Parallel Bus Decoder

```rust
use dsl_loader::StreamingParallelDecoder;
use dsl_loader::decoders::StrobeMode;

let parallel = StreamingParallelDecoder::new(
    8,                      // 8 data bits
    StrobeMode::RisingEdge, // strobe trigger
    false,                  // no enable signal
);
```

Supports:

- 1-64 data bits
- Rising/falling/any edge or level triggers
- Optional enable signal
- Outputs `ParallelWord` events

## Performance

- **Thread-per-node**: Parallel processing on multi-core systems
- **Backpressure**: Automatic flow control via bounded channels
- **On-demand I/O**: DSL files read only needed blocks

Tested at:

- 12 channels @ 250MHz
- 3 channels @ 1GHz

## Documentation

- **[API.md](API.md)** - Complete API documentation and usage guide
- **[DESIGN.md](DESIGN.md)** - Internal architecture and design decisions

## Examples

- `streaming_file_decode.rs` - File replay with parallel bus decoder
- `spi_controlled_decode.rs` - SPI command-controlled parallel decode
- `pipeline_demo.rs` - Basic pipeline construction demo

Run with:

```bashrelease --example streaming_file_decode -- --file scan.dsl
cargo run --release --example spi_controlled_decode -- --file scan.dsl [options]
```bash
cargo build --release
cargo test
cargo build --examples
```

## Dependencies

- `crossbeam-channel`: Lock-free MPSC channels
- `zip`: DSL file reading
- `thiserror`: Error handling
- `clap`: CLI parsing (examples only)

## License

[Your License Here]
