//! Native USB adaptation for the U3Pro16 processing owner.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use logic_analyzer_acquisition::{LogicAnalyzerError, LogicAnalyzerResult};
use logic_analyzer_device_dslogic::{
    DsLogicU3Pro16SourceFactory, DsLogicU3Pro16TransportFactory, LinkSpeed, UsbError, UsbTransport,
};

const VENDOR_ID: u16 = 0x2a0e;
const PRODUCT_ID: u16 = 0x002a;
const RUNTIME_MANUFACTURER: &str = "DreamSourceLab";
const RUNTIME_PRODUCT: &str = "USB-based DSL Instrument v2";

struct NativeU3Pro16Transport {
    device: platform::NativeUsbDevice,
}

impl UsbTransport for NativeU3Pro16Transport {
    fn link_speed(&self) -> LinkSpeed {
        match self.device.link_speed() {
            platform::UsbLinkSpeed::High => LinkSpeed::High,
            platform::UsbLinkSpeed::Super => LinkSpeed::Super,
        }
    }

    fn fpga_image(&self) -> LogicAnalyzerResult<Option<Vec<u8>>> {
        let Some(path) = fpga_image_candidates()
            .into_iter()
            .find(|path| path.is_file())
        else {
            return Ok(None);
        };
        let image = std::fs::read(&path).map_err(|error| {
            LogicAnalyzerError::transport_message(format!(
                "cannot read U3Pro16 FPGA image '{}': {error}",
                path.display()
            ))
        })?;
        tracing::info!(path = %path.display(), "loaded DSLogic U3Pro16 FPGA image");
        Ok(Some(image))
    }

    fn control_write(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
        timeout: Duration,
    ) -> Result<usize, UsbError> {
        self.device
            .control_write(request_type, request, value, index, data, timeout)
            .map_err(map_usb_error)
    }

    fn control_read(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        data: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, UsbError> {
        self.device
            .control_read(request_type, request, value, index, data, timeout)
            .map_err(map_usb_error)
    }

    fn bulk_write(
        &mut self,
        endpoint: u8,
        data: &[u8],
        timeout: Duration,
    ) -> Result<usize, UsbError> {
        self.device
            .bulk_write(endpoint, data, timeout)
            .map_err(map_usb_error)
    }

    fn bulk_read(
        &mut self,
        endpoint: u8,
        data: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, UsbError> {
        self.device
            .bulk_read(endpoint, data, timeout)
            .map_err(map_usb_error)
    }

    fn queue_bulk_read(
        &mut self,
        endpoint: u8,
        byte_len: usize,
        timeout: Duration,
    ) -> Result<bool, UsbError> {
        self.device
            .queue_bulk_read(endpoint, byte_len, timeout)
            .map_err(map_usb_error)
    }

    fn take_queued_bulk_read(
        &mut self,
        byte_len: usize,
        timeout: Duration,
    ) -> Result<Option<Vec<u8>>, UsbError> {
        self.device
            .take_queued_bulk_read(byte_len, timeout)
            .map_err(map_usb_error)
    }

    fn cancel_queued_bulk_read(&mut self) -> Result<(), UsbError> {
        self.device.cancel_queued_bulk_read().map_err(map_usb_error)
    }

    fn close(&mut self) -> Result<(), UsbError> {
        self.device.close().map_err(map_usb_error)
    }
}

struct NativeU3Pro16TransportFactory;

impl DsLogicU3Pro16TransportFactory for NativeU3Pro16TransportFactory {
    fn open(&self) -> LogicAnalyzerResult<Box<dyn UsbTransport>> {
        let selector = platform::NativeUsbDeviceSelector::new(VENDOR_ID, PRODUCT_ID)
            .with_identity_prefixes(RUNTIME_MANUFACTURER, RUNTIME_PRODUCT)
            .with_configuration_interface(1, 0);
        platform::NativeUsbDevice::open(&selector)
            .map(|device| Box::new(NativeU3Pro16Transport { device }) as Box<dyn UsbTransport>)
            .map_err(LogicAnalyzerError::transport)
    }
}

pub(crate) fn transport_factory() -> Arc<dyn DsLogicU3Pro16TransportFactory> {
    Arc::new(NativeU3Pro16TransportFactory)
}

pub(crate) fn source_factory() -> Arc<dyn DsLogicU3Pro16SourceFactory> {
    logic_analyzer_device_dslogic::source_factory(transport_factory())
}

fn map_usb_error(error: platform::UsbTransferError) -> UsbError {
    match error {
        platform::UsbTransferError::Timeout => UsbError::Timeout,
        platform::UsbTransferError::Other => UsbError::Other,
    }
}

fn fpga_image_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![
        PathBuf::from("DSLogicU3Pro16.bin"),
        PathBuf::from("firmware/DSLogicU3Pro16.bin"),
        PathBuf::from("/Applications/DSView.app/Contents/MacOS/res/DSLogicU3Pro16.bin"),
        PathBuf::from("/Applications/DSView.app/Contents/Resources/driver/DSLogicU3Pro16.bin"),
        PathBuf::from("/usr/share/DSView/driver/DSLogicU3Pro16.bin"),
        PathBuf::from("/usr/local/share/DSView/driver/DSLogicU3Pro16.bin"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = Path::new(&home);
        candidates.push(home.join(".local/share/DSView/driver/DSLogicU3Pro16.bin"));
        candidates.push(home.join("Library/Application Support/DSView/driver/DSLogicU3Pro16.bin"));
    }
    if let Some(path) = std::env::var_os("DSLOGIC_U3PRO16_FPGA_IMAGE") {
        candidates.push(PathBuf::from(path));
    }
    candidates
}
