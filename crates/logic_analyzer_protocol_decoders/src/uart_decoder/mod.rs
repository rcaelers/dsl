//! # `uart_decoder`
//!
//! ## Responsibility
//!
//! This module owns UART signal decoding and UART word/diagnostic production from generic sampled
//! inputs.
//!
//! ## Boundaries
//!
//! It does not define UART node sockets, panel controls, display presentation, or host scheduling.
//! Those concerns belong to the graph-node, UI, and runtime owners.

//! UART decoder processing node.
//!
//! It owns UART signal decoding and word/diagnostic production from generic sampled
//! inputs. Graph definition and presentation behavior belong to the node feature.

mod implementation;

pub use implementation::{UartDecoder, UartParity, UartStopBits};
