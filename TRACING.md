# Per-Node Tracing Guide

The DSL library provides tracing for all nodes, with log levels controlled by module path.

## How It Works

Each node is implemented in its own module, allowing you to set log levels per node type
:
- `dsl::nodes::dsl_file` - File source node
- `dsl::nodes::decoders::streaming_spi_decoder` - SPI decoder
- `dsl::nodes::decoders::streaming_parallel_decoder` - Parallel decoder
- `dsl::nodes::state` - State controllers

## Filtering Examples

### Global Level

Set all logging to info:
```bash
RUST_LOG=info ./your_program
```

### Per-Node Type Filtering

Enable different levels for different node types:
```bash
# Info globally, debug for SPI decoder
RUST_LOG=info,dsl::nodes::decoders::spi_decoder=debug ./your_program

# Multiple nodes with different levels
RUST_LOG=info,dsl::nodes::decoders::spi_decoder=debug,dsl::nodes::decoders::parallel_decoder=trace ./your_program

# All decoders at debug
RUST_LOG=info,dsl::nodes::decoders=debug ./your_program
```

### Example Output

Node logs show the module path and node name in brackets:
```
2026-02-10T17:03:34.118188Z  INFO dsl::nodes::dsl_file: [dsl_file_source] File source: Spawning per-channel threads ...
2026-02-10T17:03:34.118205Z DEBUG dsl::nodes::decoders::spi_decoder: Waiting for CS active...
2026-02-10T17:03:34.118309Z ERROR dsl::runtime::scheduler: [spi_decoder] Work error: Shutdown signal received
```

### Recommended Patterns

**Development** - Show everything:
```bash
RUST_LOG=debug ./your_program
```

**Production** - Quiet except errors:
```bash
RUST_LOG=warn ./your_program
```

**Debugging specific node type**:
```bash
# Debug just the SPI decoder
RUST_LOG=info,dsl::nodes::decoders::spi_decoder=debug ./your_program

# Trace SPI decoder, debug file source
RUST_LOG=info,dsl::nodes::dsl_file=debug,dsl::nodes::decoders::spi_decoder=trace ./your_program
```

**Module-based filtering**:
```bash
# Info for runtime, debug for all nodes
RUST_LOG=info,dsl::nodes=debug ./your_program

# Debug for all decoders, info for everything else
RUST_LOG=info,dsl::nodes::decoders=debug ./your_program
```

## Module Paths

Node types and their module paths for filtering:

| Node Type | Module Path |
|-----------|-------------|
| File source | `dsl::nodes::dsl_file` |
| SPI decoder | `dsl::nodes::decoders::spi_decoder` |
| Parallel decoder | `dsl::nodes::decoders::parallel_decoder` |
| SPI command controller | `dsl::nodes::state` |
| Pipeline/scheduler | `dsl::runtime::pipeline` or `dsl::runtime::scheduler` |

## Watchdog for Deadlock Detection

The library includes automatic deadlock detection that monitors blocking channel operations
 (recv/send). When enabled, it reports operations that have been blocked for more than 5 seconds.

### Enabling the Watchdog

Enable via the Pipeline API:
```rust
let mut pipeline = Pipeline::new();
// ... add nodes and connections ...

// Build with watchdog enabled
let scheduler = pipeline
    .with_watchdog()  // Enable deadlock detection
    .build()?;

scheduler.wait();
```

### How It Works

The watchdog integration is **transparent to node implementations**:

1. When you enable watchdog with `.with_watchdog()`, the pipeline automatically injects watchdog context into all InputPort and OutputPort instances
2. **Receiver** automatically creates `OperationGuard` during `recv()` and `peek()` operations when watchdog context is available
3. **WatchdogSender** wrapper automatically creates `OperationGuard` during `send()` operations
4. Nodes use these wrappers normally - the watchdog monitoring happens automatically

### What Gets Monitored

- **Receive operations**: `Receiver::recv()` and `peek()`
- **Send operations**: Output port sends via `WatchdogSender`
- Operations are tracked with node name and port name for clear diagnostics

### Example Watchdog Output

When a node is blocked on a channel operation for >5 seconds:
```
2026-02-10T17:03:39.118Z WARN dsl::runtime::watchdog: ⚠️  BLOCKED: [spi_decoder] recv on port 'clk' for 5.2s (op_id: 42)
2026-02-10T17:03:40.118Z WARN dsl::runtime::watchdog: ⚠️  BLOCKED: [parallel_decoder] send on port 'words_out' for 6.1s (op_id: 43)
```

This helps identify:
- Which node is stuck
- What operation (recv/send)
- Which port
- How long it's been blocked

### Using in Custom Nodes

The watchdog is transparent - just use the standard channel wrappers:

```rust
impl ProcessNode for MyNode {
    fn work(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        // Get input with automatic watchdog monitoring
        let mut input = inputs
            .first()
            .and_then(|p| p.get(&mut self.buffer))?;
        
        // Recv is automatically monitored when watchdog is enabled
        let item = input.recv()?;
        
        // Get output with automatic watchdog monitoring
        let output = outputs
            .first()
            .and_then(|p| p.get::<MyType>())?;
        
        // Send is automatically monitored when watchdog is enabled
        output.send(processed_item)?;
        
        Ok(1)
    }
}
```

No explicit `OperationGuard` creation needed - it's all handled transparently.

## Module Paths

Node types and their module paths for filtering:

| Node Type | Module Path |
|-----------|-------------|
| File source | `dsl::nodes::dsl_file` |
| SPI decoder | `dsl::nodes::decoders::spi_decoder` |
| Parallel decoder | `dsl::nodes::decoders::parallel_decoder` |
| SPI command controller | `dsl::nodes::state` |
| Pipeline/scheduler | `dsl::runtime::pipeline` or `dsl::runtime::scheduler` |

## Using in Custom Nodes

To add tracing to your own nodes:

```rust
use tracing::{debug, info, trace};

impl ProcessNode for MyNode {
    fn work(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        info!("Starting work");
        debug!("Processing item {}", self.count);
        trace!("Detailed trace information");
        
        // ... your work ...
        
        Ok(items_processed)
    }
}
```

Use standard tracing macros (`info!`, `debug!`, `trace!`, etc.) for all logging.

## Quick Reference

| Pattern | Description |
|---------|-------------|
| `RUST_LOG=debug` | Everything at debug |
| `RUST_LOG=info,dsl::nodes=debug` | Info globally, debug for all nodes |
| `RUST_LOG=info,dsl::nodes::decoders::spi_decoder=debug` | Debug for SPI decoder only |
| `RUST_LOG=info,dsl::nodes::decoders=trace` | Trace for all decoders |
| `RUST_LOG=info,dsl::nodes::dsl_file=debug` | Debug for file source only |
