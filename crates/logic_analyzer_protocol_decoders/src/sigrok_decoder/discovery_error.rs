use std::error::Error;
use std::sync::Arc;

use thiserror::Error;

/// Failure while locating or inspecting one Sigrok decoder package.
#[derive(Clone, Debug, Error)]
pub enum SigrokDecoderDiscoveryError {
    /// The host does not provide Sigrok decoder discovery.
    #[error("{0}")]
    Unavailable(String),
    /// The host could not inspect the decoder package and its declared contract.
    #[error("could not inspect Sigrok decoder package: {source}")]
    Inspection {
        /// The host-specific inspection failure.
        #[source]
        source: Arc<dyn Error + Send + Sync>,
    },
    /// The host could not fingerprint the decoder package contents.
    #[error("could not fingerprint Sigrok decoder package: {source}")]
    Fingerprint {
        /// The host-specific fingerprinting failure.
        #[source]
        source: Arc<dyn Error + Send + Sync>,
    },
    /// A host adapter supplied only a presentation-ready diagnostic.
    #[error("{0}")]
    Diagnostic(String),
}

impl SigrokDecoderDiscoveryError {
    /// Reports that the host does not provide package discovery.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable(message.into())
    }

    /// Preserves a host-specific package inspection failure.
    pub fn inspection(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Inspection {
            source: Arc::new(error),
        }
    }

    /// Preserves a host-specific package fingerprinting failure.
    pub fn fingerprint(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Fingerprint {
            source: Arc::new(error),
        }
    }

    /// Adapts a presentation-only host diagnostic.
    pub fn diagnostic(message: impl Into<String>) -> Self {
        Self::Diagnostic(message.into())
    }
}

/// Failure that prevents a host from producing any Sigrok catalog snapshot.
///
/// Missing paths, unreadable paths, and invalid individual decoder packages are recoverable
/// per-entry diagnostics in `SigrokCatalogSnapshot`, rather than whole-scan failures.
#[derive(Clone, Debug, Error)]
pub enum SigrokCatalogError {
    /// The host discovery mechanism could not produce a catalog snapshot.
    #[error("Sigrok decoder catalog scan failed: {source}")]
    Scan {
        /// The host-specific scan failure.
        #[source]
        source: Arc<dyn Error + Send + Sync>,
    },
    /// A host adapter supplied only a presentation-ready diagnostic.
    #[error("Sigrok decoder catalog scan failed: {0}")]
    Diagnostic(String),
}

impl SigrokCatalogError {
    /// Preserves a host-specific whole-catalog scan failure.
    pub fn scan(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Scan {
            source: Arc::new(error),
        }
    }

    /// Adapts a presentation-only host diagnostic.
    pub fn diagnostic(message: impl Into<String>) -> Self {
        Self::Diagnostic(message.into())
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::io;

    use super::{SigrokCatalogError, SigrokDecoderDiscoveryError};

    #[test]
    fn decoder_discovery_preserves_the_host_source() {
        let error = SigrokDecoderDiscoveryError::inspection(io::Error::other("inspection failed"));

        assert!(matches!(
            &error,
            SigrokDecoderDiscoveryError::Inspection { .. }
        ));
        assert_eq!(error.source().unwrap().to_string(), "inspection failed");
    }

    #[test]
    fn fatal_catalog_scan_preserves_the_host_source() {
        let error = SigrokCatalogError::scan(io::Error::other("scan failed"));

        assert!(matches!(&error, SigrokCatalogError::Scan { .. }));
        assert_eq!(error.source().unwrap().to_string(), "scan failed");
    }
}
