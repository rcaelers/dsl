//! Explicit hardware validation owned by the U3Pro16 source.

use std::path::Path;

use logic_analyzer_acquisition::{LogicAnalyzer, LogicCaptureConfig};

use super::implementation::DsLogicU3Pro16;
use super::transport::DsLogicU3Pro16TransportFactory;

/// Loads an explicit FPGA image into a connected U3Pro16 and verifies its HDL version.
///
/// # Parameters
/// - `transport_factory`: Input consumed by this operation.
/// - `image_path`: Input consumed by this operation.
pub fn validate_fpga_hardware(
    transport_factory: &dyn DsLogicU3Pro16TransportFactory,
    image_path: &Path,
) -> Result<(), String> {
    let image = std::fs::read(image_path)
        .map_err(|error| format!("cannot read FPGA image '{}': {error}", image_path.display()))?;
    let mut analyzer = DsLogicU3Pro16::new(
        transport_factory
            .open()
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    analyzer
        .configure_fpga(&image)
        .map_err(|error| error.to_string())?;
    let version = analyzer
        .logic_version()
        .map_err(|error| error.to_string())?;
    if version != 0x0e {
        return Err(format!(
            "configured U3Pro16 reported HDL version {version:#04x}, expected 0x0e"
        ));
    }
    println!("U3Pro16 FPGA validation passed with HDL version {version:#04x}");
    Ok(())
}

/// Captures 1,024 samples and verifies that the trigger header precedes logic data.
pub fn validate_capture_hardware(
    transport_factory: &dyn DsLogicU3Pro16TransportFactory,
) -> Result<(), String> {
    let config = LogicCaptureConfig::finite(1_000_000, 0b11, 1_024);
    let mut analyzer = DsLogicU3Pro16::new(
        transport_factory
            .open()
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    analyzer
        .configure_capture(&config)
        .map_err(|error| error.to_string())?;
    analyzer
        .start_capture()
        .map_err(|error| error.to_string())?;
    let capture_result = analyzer.next_chunk().map_err(|error| error.to_string());
    let header_present = analyzer.take_trigger_header().is_some();
    let stop_result = analyzer.stop_capture().map_err(|error| error.to_string());
    capture_result?;
    stop_result?;
    if !header_present {
        return Err("capture produced logic data before its trigger header".into());
    }
    println!("U3Pro16 capture validation received the trigger header before logic data");
    Ok(())
}
