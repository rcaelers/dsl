//! Native embedded-Python host for Sigrok decoder packages.

#[allow(dead_code)]
mod bridge;
#[allow(dead_code)]
mod conditions;
mod discovery;
mod execution;
mod python_error;
#[allow(dead_code)]
mod python_host;
mod runtime;
#[allow(dead_code)]
mod scheduler;
#[cfg(feature = "developer-tools")]
#[allow(dead_code)]
mod upstream_validation;
#[allow(dead_code)]
mod worker;

#[cfg(test)]
mod worker_tests;

pub(crate) use bridge::DecoderOutput;
pub(crate) use discovery::{discover_sigrok_decoder, scan_catalog};
pub(crate) use execution::PythonSigrokExecutionFactory;
pub(crate) use runtime::{catalog_scanner, decoder_runtime};
#[cfg(feature = "developer-tools")]
#[allow(unused_imports)]
pub(crate) use upstream_validation::{validate_spi_chunk_boundaries, validate_spi_oracle};
pub(crate) use worker::{DecoderWorker, OptionValue, WorkerConfig, WorkerInputConfig};
