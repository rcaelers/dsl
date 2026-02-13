# DSL Logic Analyzer Streaming Decoder

A high-performance streaming architecture for DSLogic logic analyzer signal processing in Rust.

## Features

- **Thread-per-node parallel processing** using crossbeam channels
- **Generic channel support**: 1-16 channels with bit-packing for bandwidth reduction
- **Full protocol decoders**: SPI and parallel bus decoders
- **DSL file source**: Efficient replay of DSLogic capture files
- **Type-safe pipeline builder**: Named ports with compile-time type checking

## Architecture

The system uses a thread-per-node streaming architecture:

```text
┌──────────────────────┐     ┌─────────────┐     ┌───────┐
│   DslFileSource      │────▶│  SpiDecoder │────▶│ Sink  │
│      (Thread 1)      │     │  (Thread 2) │     │(Thr 3)│
└──────────────────────┘     └─────────────┘     └───────┘
       Sample×N              SpiTransfer          Print
```

Each node runs in its own thread, connected by bounded crossbeam channels for automatic backpressure.

## Quick Start

### Basic File Replay

```rust
use dsl::Pipeline;
use dsl::nodes::DslFileSource;

// Create pipeline
let mut pipeline = Pipeline::new();

// Add file source (16 channels)
pipeline.add_process("source", DslFileSource::new("capture.dsl", 16)?)?;

// Add decoder
pipeline.add_process("spi", SpiDecoder::new(
    SpiMode::Mode0,
    24,     // bits per word
    true,   // has MOSI
    false,  // no MISO
))?;

// Connect using named ports
pipeline.connect("source", "d7", "spi", "clk")?;
pipeline.connect("source", "d8", "spi", "cs")?;
pipeline.connect("source", "d6", "spi", "mosi")?;

// Build and run
let scheduler = pipeline.build()?;
scheduler.wait();
```

See [examples/spi_controlled_decode.rs](examples/spi_controlled_decode.rs) for a complete example with multi-file output.

## Core Components

### Sample Type

- **`Sample`**: Boolean signal with timestamp (value + start_time)
  - Used for all individual channel data

### Protocol Types

- **`SpiTransfer`**: Decoded SPI transaction (mosi, miso, timing info)
- **`ParallelWord`**: Decoded parallel bus word (value, timing info)

### Nodes

All nodes implement the `ProcessNode` trait:

- **Sources**: `DslFileSource` (0 inputs, N outputs)
- **Processors**: `SpiDecoder`, `ParallelDecoder` (N inputs, M outputs)
- **Sinks**: Custom nodes for output/analysis (N inputs, 0 outputs)

### Runtime

- **`Pipeline`**: Type-safe graph construction with named ports
- **`Scheduler`**: Thread-per-node execution with automatic watchdog monitoring
- **`ProcessNode`**: Core trait defining node behavior

## Protocol Decoders

### SPI Decoder

```rust
use dsl::nodes::decoders::{SpiDecoder, SpiMode};

let spi = SpiDecoder::new(
    SpiMode::Mode0,  // CPOL=0, CPHA=0
    24,              // bits per word
    true,            // has MOSI
    false,           // no MISO (3 inputs: CLK, CS, MOSI)
);
```

**Features:**
- All SPI modes (0-3)
- Configurable word size (1-64 bits)
- MOSI/MISO optional
- CS active-low detection

**Inputs:** CLK, CS, MOSI (optional), MISO (optional)  
**Output:** `SpiTransfer` events

### Parallel Bus Decoder

```rust
use dsl::nodes::decoders::{ParallelDecoder, StrobeMode};

let parallel = ParallelDecoder::new(
    8,                       // 8 data bits
    StrobeMode::RisingEdge,  // strobe trigger
    true,                    // cs_active_low
);
```

**Features:**
- 1-64 data bits
- Rising/falling/any edge or level triggers
- Enable signal support
- CS signal for gating

**Inputs:** strobe, d0..dN, enable_signal, cs  
**Output:** `ParallelWord` events

## Building & Testing

```bash
# Build (release mode recommended for performance)
cargo build --release

# Run tests
cargo test

# Run examples (always use --release for file processing)
cargo run --release --example spi_controlled_decode -- --file scan.dsl
```

## Examples

- **`spi_decode.rs`** - Basic SPI decoding with CSV output
- **`spi_controlled_decode.rs`** - SPI-controlled parallel decode with multi-file capture

Run with debug logging:
```bash
RUST_LOG=debug cargo run --release --example spi_decode
```

## Performance

- **Parallel processing**: Each node runs on its own CPU core
- **Lock-free channels**: Crossbeam MPSC with minimal contention
- **Bit-packing**: 12ch @ 250MHz = 375MB/s raw → 47MB/s packed (8x reduction)
- **Automatic backpressure**: Bounded channels prevent memory overflow

**Tested configurations:**
- 12 channels @ 250MHz
- 16 channels @ 200MHz

## Documentation

- **[API.md](API.md)** - Complete API documentation
- **[DESIGN.md](DESIGN.md)** - Architecture and design decisions
- **[TRACING.md](TRACING.md)** - Watchdog and debug logging guide

## Dependencies

- `crossbeam-channel` - Lock-free MPSC channels
- `zip` - DSL file reading
- `thiserror` - Error handling
- `tracing` - Structured logging with watchdog support
- `clap` - CLI parsing (examples only)

## License

MIT - see [LICENSE](LICENSE) file for details
