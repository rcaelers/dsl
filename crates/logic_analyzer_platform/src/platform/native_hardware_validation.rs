//! Native U3Pro16 validation commands composed with the host USB adapter.

use std::path::Path;

use logic_analyzer_processing::nodes::sources::dslogic_u3pro16::{
    validate_capture_hardware as validate_capture_hardware_with_transport,
    validate_fpga_hardware as validate_fpga_hardware_with_transport,
};

use super::native::native_u3pro16_transport_factory;

/// Loads an explicit FPGA image and verifies its reported HDL version.
///
/// # Parameters
/// - `image_path`: Input consumed by this operation.
pub fn validate_fpga_hardware(image_path: &Path) -> Result<(), String> {
    validate_fpga_hardware_with_transport(native_u3pro16_transport_factory().as_ref(), image_path)
}

/// Captures a small U3Pro16 trace and verifies trigger-header ordering.
pub fn validate_capture_hardware() -> Result<(), String> {
    validate_capture_hardware_with_transport(native_u3pro16_transport_factory().as_ref())
}
