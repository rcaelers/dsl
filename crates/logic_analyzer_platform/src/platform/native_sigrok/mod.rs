//! Native embedded-Python host for Sigrok decoder packages.

#[allow(dead_code)]
mod bridge;
mod catalog;
#[allow(dead_code)]
mod conditions;
mod discovery;
mod execution;
mod python_error;
#[allow(dead_code)]
mod python_host;
#[allow(dead_code)]
mod scheduler;
#[cfg(feature = "developer-tools")]
mod upstream_validation;
#[allow(dead_code)]
mod worker;

#[cfg(test)]
mod worker_tests;

pub(crate) use bridge::DecoderOutput;
pub(crate) use catalog::directory_catalog;
pub(crate) use discovery::{discover_sigrok_decoder, scan_catalog};
pub(crate) use execution::PythonSigrokExecutionFactory;
#[cfg(feature = "developer-tools")]
pub use upstream_validation::{validate_spi_chunk_boundaries, validate_spi_oracle};
pub(crate) use worker::{DecoderWorker, OptionValue, WorkerConfig, WorkerInputConfig};
