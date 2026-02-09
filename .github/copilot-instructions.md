# DSL Loader Agent Instructions

## Documentation Style

**IMPORTANT**: Do not create historical documentation, migration guides, or changelog files. When making improvements to the codebase:
- Update existing documentation to reflect the current state only
- Remove references to deprecated patterns
- Do not document "before and after" comparisons
- Keep documentation focused on how things work now
- **After refactoring, replace old implementations entirely** - do not keep compatibility functions, old versions, or "legacy" code paths
- **Do not add convenience wrapper functions** - use the canonical API directly (e.g., `add_process()` not `add_source()` or `add_sink()`)
- Remove old files and rename new ones to canonical names (e.g., remove "_v2" or "_simplified" suffixes)

## Architecture Overview

This is a **streaming dataflow architecture** for logic analyzer signal processing with thread-per-node parallel processing and typed channel connections.

**Core concept**: `DslFileSource → Decoders → Sinks`, where each node runs in its own thread connected by crossbeam channels.

Key files to understand the architecture:
- [DESIGN.md](../DESIGN.md) - internal architecture decisions
- [API.md](../API.md) - complete API documentation
- [src/runtime/ports.rs](../src/runtime/ports.rs) - Pipeline API
- [src/runtime/work.rs](../src/runtime/work.rs) - node trait definitions

## Node System

### ProcessNode - The Only Node Type

All nodes implement the `ProcessNode` trait:
- **Sources** have 0 inputs and N outputs (e.g., `DslFileSource`)
- **Sinks** have N inputs and 0 outputs (e.g., printers, loggers, analysis nodes)
- **Processors** have N inputs and M outputs (e.g., `SpiDecoder`, `SpiCommandController`)

**Method signatures**:
- `fn name(&self) -> &str` - Debug name for this node
- `fn should_stop(&self) -> bool` - Whether node should stop (default: false)
- `fn num_inputs(&self) -> usize` - Report how many input ports the node requires
- `fn num_outputs(&self) -> usize` - Report how many output ports the node provides
- `fn work(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize>` - Do the actual work

**Critical**: 
- Nodes determine their own port counts based on their configuration
- Return `WorkResult<usize>` where the number is items produced/consumed
- Return `Err(WorkError::Shutdown)` to cleanly stop

### Work Function Patterns

**Source (0 inputs, N outputs)**:
```rust
impl ProcessNode for MySource {
    fn num_inputs(&self) -> usize { 0 }  // Source
    fn num_outputs(&self) -> usize { self.num_channels as usize }
    
    fn work(&mut self, _inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        let output = outputs
            .first()
            .and_then(|p| p.get::<OutputType>())
            .ok_or_else(|| WorkError::NodeError("Missing output".to_string()))?;
        
        // Generate data...
        output.send(data)?;
        
        Ok(1)  // Return count of items produced
    }
}
```

**Sink (N inputs, 0 outputs)**:
```rust
impl ProcessNode for MySink {
    fn num_inputs(&self) -> usize { 1 }
    fn num_outputs(&self) -> usize { 0 }  // Sink
    
    fn work(&mut self, inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
        // InputPort.get() returns a Receiver - need a buffer
        let mut input = inputs
            .first()
            .and_then(|p| p.get::<InputType>(&mut self.buffer))
            .ok_or_else(|| WorkError::NodeError("Missing input".to_string()))?;
        
        let data = input.recv()?;  // Blocks until data available
        // Process data...
        
        Ok(1)  // Return count of items consumed
    }
}
```

**Processor (N inputs, M outputs)**:
```rust
impl ProcessNode for MyProcessor {
    fn num_inputs(&self) -> usize { 1 }
    fn num_outputs(&self) -> usize { 1 }
    
    fn work(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        let mut input = inputs[0].get::<InputType>(&mut self.input_buffer).unwrap();
        let output = outputs[0].get::<OutputType>().unwrap();
        
        let data = input.recv()?;
        // Transform data...
        output.send(result)?;
        
        Ok(1)
    }
}
```

## Pipeline API (Preferred)

Use the **Pipeline API** for new code - it's type-safe and ergonomic:

```rust
let mut pipeline = Pipeline::new();

// Add nodes by name (node determines its own input/output counts)
pipeline.add_process(
    "source",
    DslFileSource::new("file.dsl", 16)?,
)?;

pipeline.add_process("sink", MySink::new())?;

// Connect using node names and port names
pipeline.connect("source", "d0", "sink", "input")?;

// Build and run
let scheduler = pipeline.build()?;
scheduler.wait();
```

### Multi-Port Connections

For nodes with multiple ports (like SPI decoders), connect each port individually:

```rust
pipeline.connect("source", "d7", "spi", "clk")?;
pipeline.connect("source", "d8", "spi", "cs")?;
pipeline.connect("source", "d6", "spi", "mosi")?;
// Optional: pipeline.connect("source", "d5", "spi", "miso")?;
```

## Data Types

### Sample Types

- **BoolSample**: Single boolean channel with timestamp
  - Created: `BoolSample::new(value: bool, timestamp: u64)`
  - Used for all signal channels in decoders and file sources

### Protocol Events

- **SpiTransfer**: `{ mosi: u64, miso: u64, timestamp: u64, position: u64 }`
- **ParallelWord**: `{ value: u64, timestamp: u64, position: u64 }`

## Logging Convention

**Always use `tracing`**, never `println!`:

```rust
use tracing::{info, debug, warn, error};

info!("Major events");      // Pipeline progress, file open
debug!("Detailed traces");   // Per-sample processing (use sparingly)
warn!("Recoverable issues"); // Unexpected but handled
error!("Critical failures"); // Before returning Err
```

Initialize in examples:
```rust
tracing_subscriber::fmt()
    .with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
    )
    .init();
```

Run with: `RUST_LOG=debug cargo run --example foo`

## Buffer Sizing Strategy

Default buffer is 1000 items. Adjust based on bandwidth:

- **High-bandwidth** (BoolSample streams): 10K-10M samples
  ```rust
  pipeline.with_default_buffer_size(10_000_000);
  // or per-connection:
  pipeline.connect_with_buffer("source", "d0", "decoder", "input", 5_000_000)?;
  ```

- **Low-bandwidth** (decoded events): 1K-10K items

**Why large buffers**: DslFileSource broadcasts to N output channels sequentially with blocking sends. If any consumer is slow, it creates head-of-line blocking. Large buffers prevent deadlock.

## Common Patterns

### File Replay Pipeline

```rust
pipeline.add_process(
    "source",
    DslFileSource::new(path, num_channels)?,
)?;
```

### SPI Decoding

```rust
pipeline.add_process(
    "spi",
    SpiDecoder::new(
        SpiMode::Mode0, // CPOL=0, CPHA=0
        24,             // bits per word
        true,           // has_mosi
        false,          // has_miso (false = only 3 inputs: CLK, CS, MOSI)
    ),
)?;

// Wire from file source using named ports
pipeline.connect("source", "d7", "spi", "clk")?;
pipeline.connect("source", "d8", "spi", "cs")?;
pipeline.connect("source", "d6", "spi", "mosi")?;
```

**Data flow**: `BoolSample` inputs → `SpiTransfer` output

### Parallel Bus Decoding

```rust
pipeline.add_process(
    "parallel",
    ParallelDecoder::new(
        8,                       // data width
        StrobeMode::RisingEdge,  // when to capture
        true,                    // has enable signal
    ),
)?;

// First input = strobe, then data bits in order, last = enable (if has_enable)
pipeline.connect("source", "d10", "parallel", "strobe")?;
for (i, ch) in [0,1,2,3,4,5,6,7].iter().enumerate() {
    pipeline.connect("source", &format!("d{}", ch), "parallel", &format!("d{}", i))?;
}
pipeline.connect("source", "d9", "parallel", "enable")?;
```

**Data flow**: `BoolSample` inputs → `ParallelWord` output

### SPI-Controlled Decoding

Use `SpiCommandController` to enable/disable based on SPI commands:

```rust
pipeline.add_process(
    "controller",
    SpiCommandController::new(0x600081, 0x600000),  // enable_cmd, disable_cmd
)?;

pipeline.connect("spi", "spi_transfers", "controller", "spi_in")?;
pipeline.connect("controller", "enable_signal", "parallel_decoder", "enable")?;
```

See [examples/spi_controlled_decode.rs](../examples/spi_controlled_decode.rs) for full example.

## Building & Testing

```bash
# Build (release mode for performance)
cargo build --release

# Run tests
cargo test

# Run clippy (REQUIRED after every code change)
cargo clippy --all-targets

# Run examples (always use --release for file processing)
cargo run --release --example streaming_file_decode -- --file scan.dsl

# With logging
RUST_LOG=debug cargo run --release --example pipeline_demo
```

**Important**: 
- Always use `--release` for examples that process files. Debug builds are 10-100x slower.
- **Always run `cargo clippy --all-targets` after every code change** to ensure no warnings are introduced. This is required for every edit, even without explicit instruction.

## Type System

### Port Type Safety

Connections are type-checked at connection time:

```rust
// ✅ Valid: BoolSample → BoolSample
pipeline.connect("source", "d0", "decoder", "clk")?;

// ❌ Runtime error: type mismatch caught at connection time
pipeline.connect("source", "d0", "spi_printer", "spi_input")?;  // BoolSample → SpiTransfer
```

### Generic Channel Count

DslFileSource supports 1-16 channels dynamically:

```rust
// 8 channels
pipeline.add_process("source", DslFileSource::new("file.dsl", 8)?)?;

// 16 channels
pipeline.add_process("source", DslFileSource::new("file.dsl", 16)?)?;
```

Validation is runtime - panics if `num_channels < 1` or `> 16`.

## Error Handling

Return `WorkError` from work functions:

- `WorkError::Shutdown` - clean shutdown (not an error)
- `WorkError::RecvError` - input channel closed
- `WorkError::SendError` - output channel closed
- `WorkError::NodeError(msg)` - node-specific issues

Channels automatically convert:
```rust
let data = input.recv()?;  // RecvError → WorkError
output.send(data)?;         // SendError → WorkError
```

## Performance Notes

- **Bit-packing**: 12ch @ 250MHz = 375MB/s raw → 47MB/s packed (8x reduction)
- **Thread-per-node**: Each node runs on its own CPU core
- **Lock-free channels**: Crossbeam MPSC for minimal contention
- **Backpressure**: Automatic via bounded channels

Monitor performance: `info!()` log messages include sample counts and byte rates.

## Common Pitfalls

1. **Don't use println!** - Use `tracing::info!()` or `tracing::debug!()`
2. **Return item counts from work()** - Not just `Ok(())`, return `Ok(count)`
3. **Check num_channels matches file** - DslFileSource validates at construction
4. **SPI decoder inputs** - `has_miso=false` → 3 inputs, `has_miso=true` → 4 inputs
5. **Parallel decoder input order** - strobe first, then data bits in order, then enable last
6. **Use --release for file processing** - Debug builds are 10-100x slower

## Next Steps for New Features

1. **New decoder**: Implement `ProcessNode` with `num_inputs()`, `num_outputs()`, and `work()` methods
2. **New source**: Implement `ProcessNode` with `num_inputs() { 0 }` and `num_outputs()` returning channel count
3. **New sink**: Implement `ProcessNode` with `num_inputs()` and `num_outputs() { 0 }`
4. **Add extension trait for ports**: Create `YourNodePorts` trait with named port methods
