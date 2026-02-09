//! Node-based signal processing system
//!
//! This module provides a streaming node graph system for real-time signal processing:
//! - **Nodes**: Computation units that process samples
//! - **Channels**: Crossbeam channels for inter-node communication
//! - **Scheduler**: Thread-per-node runtime for parallel execution
//! - **Decoders**: Protocol decoders (SPI, parallel bus)
//!
//! # Architecture
//!
//! The streaming architecture uses thread-per-node execution:
//! - Source nodes produce samples (files)
//! - Process nodes transform data (decoders)
//! - Sink nodes consume results (printers, analyzers)
//! - All connected via crossbeam MPSC channels
//!
//! # Examples
//!
//! ```ignore
//! use dsl_loader::{DslFileSource, Scheduler};
//!
//! let source = DslFileSource::new("capture.dsl", 12)?;
//! // ... set up channels and run with Scheduler
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod decoders;
mod dsl_file;

// Export DslFileSource and related types for file I/O
pub use dsl_file::{DslFileSource, DslHeader};

// Re-export Sample from runtime
pub use crate::runtime::Sample;
