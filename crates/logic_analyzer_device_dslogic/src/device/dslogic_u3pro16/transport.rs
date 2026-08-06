//! Transport capability contract for the U3Pro16 device protocol.

use std::time::Duration;

use signal_capture_session::logic_analyzer::LogicAnalyzerResult;

/// USB link speed relevant to U3Pro16 capture planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkSpeed {
    /// USB 2 high-speed transport.
    High,
    /// USB 3 SuperSpeed transport.
    Super,
}

/// Failure reported by a [`UsbTransport`] operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbError {
    /// The operation did not complete before its deadline.
    Timeout,
    /// The transport failed for a reason other than a timeout.
    Other,
}

/// USB operations required by the U3Pro16 device protocol.
///
/// Implementations preserve call order. The queued-read methods may use an
/// asynchronous backend; transports without that capability retain the
/// synchronous fallback supplied by this contract.
pub trait UsbTransport: Send + 'static {
    /// Returns the negotiated USB link speed.
    fn link_speed(&self) -> LinkSpeed;
    /// Loads the host-provided FPGA image when the device requires configuration.
    fn fpga_image(&self) -> LogicAnalyzerResult<Option<Vec<u8>>> {
        Ok(None)
    }
    /// Performs one USB control write.
    ///
    /// # Parameters
    /// - `request_type`: Input consumed by this operation.
    /// - `request`: Input consumed by this operation.
    /// - `value`: Input consumed by this operation.
    /// - `index`: Input consumed by this operation.
    /// - `data`: Input consumed by this operation.
    /// - `timeout`: Input consumed by this operation.
    fn control_write(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
        timeout: Duration,
    ) -> Result<usize, UsbError>;
    /// Performs one USB control read.
    fn control_read(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        data: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, UsbError>;
    /// Writes one bulk transfer.
    fn bulk_write(
        &mut self,
        endpoint: u8,
        data: &[u8],
        timeout: Duration,
    ) -> Result<usize, UsbError>;
    /// Reads one bulk transfer.
    fn bulk_read(
        &mut self,
        endpoint: u8,
        data: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, UsbError>;
    /// Queues one bulk receive before a device command that produces its
    /// response. Implementations without asynchronous USB support return
    /// `Ok(false)` and callers fall back to a synchronous receive.
    fn queue_bulk_read(
        &mut self,
        _endpoint: u8,
        _byte_len: usize,
        _timeout: Duration,
    ) -> Result<bool, UsbError> {
        Ok(false)
    }
    /// Takes the queued receive, waiting up to `timeout` for completion.
    /// `Ok(None)` means no receive was queued.
    fn take_queued_bulk_read(
        &mut self,
        _byte_len: usize,
        _timeout: Duration,
    ) -> Result<Option<Vec<u8>>, UsbError> {
        Ok(None)
    }
    /// Cancels an outstanding queued bulk read, if present.
    fn cancel_queued_bulk_read(&mut self) -> Result<(), UsbError> {
        Ok(())
    }
    /// Releases transport resources. Implementations allow repeated calls.
    fn close(&mut self) -> Result<(), UsbError> {
        Ok(())
    }
}

impl<T: UsbTransport + ?Sized> UsbTransport for Box<T> {
    fn link_speed(&self) -> LinkSpeed {
        (**self).link_speed()
    }

    fn fpga_image(&self) -> LogicAnalyzerResult<Option<Vec<u8>>> {
        (**self).fpga_image()
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
        (**self).control_write(request_type, request, value, index, data, timeout)
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
        (**self).control_read(request_type, request, value, index, data, timeout)
    }

    fn bulk_write(
        &mut self,
        endpoint: u8,
        data: &[u8],
        timeout: Duration,
    ) -> Result<usize, UsbError> {
        (**self).bulk_write(endpoint, data, timeout)
    }

    fn bulk_read(
        &mut self,
        endpoint: u8,
        data: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, UsbError> {
        (**self).bulk_read(endpoint, data, timeout)
    }

    fn queue_bulk_read(
        &mut self,
        endpoint: u8,
        byte_len: usize,
        timeout: Duration,
    ) -> Result<bool, UsbError> {
        (**self).queue_bulk_read(endpoint, byte_len, timeout)
    }

    fn take_queued_bulk_read(
        &mut self,
        byte_len: usize,
        timeout: Duration,
    ) -> Result<Option<Vec<u8>>, UsbError> {
        (**self).take_queued_bulk_read(byte_len, timeout)
    }

    fn cancel_queued_bulk_read(&mut self) -> Result<(), UsbError> {
        (**self).cancel_queued_bulk_read()
    }

    fn close(&mut self) -> Result<(), UsbError> {
        (**self).close()
    }
}

/// Opens a U3Pro16 USB transport for a host-provided device adapter.
pub trait DsLogicU3Pro16TransportFactory: Send + Sync {
    /// Opens an accessible runtime U3Pro16 transport.
    fn open(&self) -> LogicAnalyzerResult<Box<dyn UsbTransport>>;
}
