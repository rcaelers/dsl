use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rusb::{Context, DeviceHandle, UsbContext};

/// USB link speed exposed by the native adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbLinkSpeed {
    /// USB 2 high-speed transport.
    High,
    /// USB 3 SuperSpeed or faster transport.
    Super,
}

/// Failure reported by a native USB transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbTransferError {
    /// The operation did not complete before its deadline.
    Timeout,
    /// The transport failed for another reason.
    Other,
}

const USB_CANCELLATION_TIMEOUT: Duration = Duration::from_millis(1_000);

/// Native `rusb` adapter for generic USB control and bulk transfers.
pub struct NativeUsbDevice {
    context: Context,
    handle: DeviceHandle<Context>,
    speed: UsbLinkSpeed,
    interface: u8,
    claimed: bool,
    queued_bulk_reads: VecDeque<QueuedBulkRead>,
}

struct QueuedBulkRead {
    transfer: *mut rusb::ffi::libusb_transfer,
    buffer: Box<[u8]>,
    complete: Box<AtomicBool>,
}

// A single caller owns each native transport and its queued requests.
unsafe impl Send for QueuedBulkRead {}

extern "system" fn mark_bulk_read_complete(transfer: *mut rusb::ffi::libusb_transfer) {
    // SAFETY: `user_data` points to the completion flag owned by the queued
    // request until that completed request is freed.
    unsafe {
        let complete = (*transfer).user_data.cast::<AtomicBool>();
        (*complete).store(true, Ordering::Release);
    }
}

/// Native USB device selection criteria.
pub struct NativeUsbDeviceSelector {
    vendor_id: u16,
    product_id: u16,
    manufacturer_prefix: Option<String>,
    product_prefix: Option<String>,
    configuration: u8,
    interface: u8,
}

impl NativeUsbDeviceSelector {
    /// Selects one USB device by vendor and product identifier.
    pub fn new(vendor_id: u16, product_id: u16) -> Self {
        Self {
            vendor_id,
            product_id,
            manufacturer_prefix: None,
            product_prefix: None,
            configuration: 1,
            interface: 0,
        }
    }

    /// Requires manufacturer and product strings with the supplied prefixes.
    pub fn with_identity_prefixes(
        mut self,
        manufacturer_prefix: impl Into<String>,
        product_prefix: impl Into<String>,
    ) -> Self {
        self.manufacturer_prefix = Some(manufacturer_prefix.into());
        self.product_prefix = Some(product_prefix.into());
        self
    }

    /// Selects the active configuration and claimed interface.
    pub fn with_configuration_interface(mut self, configuration: u8, interface: u8) -> Self {
        self.configuration = configuration;
        self.interface = interface;
        self
    }
}

impl NativeUsbDevice {
    /// Opens the first accessible USB device matching the selector.
    pub fn open(selector: &NativeUsbDeviceSelector) -> Result<Self, String> {
        let context = Context::new().map_err(|error| error.to_string())?;
        let devices = context.devices().map_err(|error| error.to_string())?;
        for device in devices.iter() {
            let descriptor = device
                .device_descriptor()
                .map_err(|error| error.to_string())?;
            if descriptor.vendor_id() != selector.vendor_id
                || descriptor.product_id() != selector.product_id
            {
                continue;
            }
            let speed = match device.speed() {
                rusb::Speed::High => UsbLinkSpeed::High,
                rusb::Speed::Super | rusb::Speed::SuperPlus => UsbLinkSpeed::Super,
                _ => continue,
            };
            let handle = device.open().map_err(|error| error.to_string())?;
            if let Some(prefix) = &selector.manufacturer_prefix {
                let manufacturer = handle
                    .read_manufacturer_string_ascii(&descriptor)
                    .map_err(|error| error.to_string())?;
                if !manufacturer.starts_with(prefix) {
                    continue;
                }
            }
            if let Some(prefix) = &selector.product_prefix {
                let product = handle
                    .read_product_string_ascii(&descriptor)
                    .map_err(|error| error.to_string())?;
                if !product.starts_with(prefix) {
                    continue;
                }
            }
            if handle
                .active_configuration()
                .map_err(|error| error.to_string())?
                != selector.configuration
            {
                handle
                    .set_active_configuration(selector.configuration)
                    .map_err(|error| error.to_string())?;
            }
            if handle
                .kernel_driver_active(selector.interface)
                .unwrap_or(false)
            {
                let _ = handle.detach_kernel_driver(selector.interface);
            }
            handle
                .claim_interface(selector.interface)
                .map_err(|error| error.to_string())?;
            return Ok(Self {
                context,
                handle,
                speed,
                interface: selector.interface,
                claimed: true,
                queued_bulk_reads: VecDeque::new(),
            });
        }
        Err(format!(
            "no accessible USB device {:04x}:{:04x} matched the selector",
            selector.vendor_id, selector.product_id
        ))
    }
}

impl NativeUsbDevice {
    /// Reports the negotiated USB link-speed class.
    pub fn link_speed(&self) -> UsbLinkSpeed {
        self.speed
    }

    /// Performs one USB control write transfer.
    pub fn control_write(
        &mut self,
        ty: u8,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
        timeout: Duration,
    ) -> Result<usize, UsbTransferError> {
        self.handle
            .write_control(ty, request, value, index, data, timeout)
            .map_err(native_usb_error)
    }

    /// Performs one USB control read transfer.
    pub fn control_read(
        &mut self,
        ty: u8,
        request: u8,
        value: u16,
        index: u16,
        data: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, UsbTransferError> {
        self.handle
            .read_control(ty, request, value, index, data, timeout)
            .map_err(native_usb_error)
    }

    /// Performs one synchronous USB bulk write transfer.
    pub fn bulk_write(
        &mut self,
        endpoint: u8,
        data: &[u8],
        timeout: Duration,
    ) -> Result<usize, UsbTransferError> {
        self.handle
            .write_bulk(endpoint, data, timeout)
            .map_err(native_usb_error)
    }

    /// Performs one synchronous USB bulk read transfer.
    pub fn bulk_read(
        &mut self,
        endpoint: u8,
        data: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, UsbTransferError> {
        self.handle
            .read_bulk(endpoint, data, timeout)
            .map_err(native_usb_error)
    }

    /// Queues one asynchronous USB bulk read.
    pub fn queue_bulk_read(
        &mut self,
        endpoint: u8,
        byte_len: usize,
        _timeout: Duration,
    ) -> Result<bool, UsbTransferError> {
        if self.queued_bulk_reads.len() == 8 {
            return Err(UsbTransferError::Other);
        }
        let mut buffer = vec![0; byte_len].into_boxed_slice();
        let complete = Box::new(AtomicBool::new(false));
        // SAFETY: the request and all referenced allocations stay owned by
        // `QueuedBulkRead` until the request completes or is cancelled.
        let transfer = unsafe { rusb::ffi::libusb_alloc_transfer(0) };
        if transfer.is_null() {
            return Err(UsbTransferError::Other);
        }
        unsafe {
            rusb::ffi::libusb_fill_bulk_transfer(
                transfer,
                self.handle.as_raw(),
                endpoint,
                buffer.as_mut_ptr(),
                i32::try_from(byte_len).map_err(|_| UsbTransferError::Other)?,
                mark_bulk_read_complete,
                (&raw const *complete).cast_mut().cast(),
                // Completion is polled by `take_queued_bulk_read` rather than
                // timing out inside libusb.
                0,
            );
            if rusb::ffi::libusb_submit_transfer(transfer) != 0 {
                rusb::ffi::libusb_free_transfer(transfer);
                return Err(UsbTransferError::Other);
            }
        }
        self.queued_bulk_reads.push_back(QueuedBulkRead {
            transfer,
            buffer,
            complete,
        });
        tracing::debug!(endpoint, byte_len, "queued USB bulk receive");
        Ok(true)
    }

    /// Waits for and removes one queued read with the requested byte length.
    pub fn take_queued_bulk_read(
        &mut self,
        byte_len: usize,
        timeout: Duration,
    ) -> Result<Option<Vec<u8>>, UsbTransferError> {
        if !self
            .queued_bulk_reads
            .iter()
            .any(|queued| queued.buffer.len() == byte_len)
        {
            tracing::debug!("no queued USB bulk receive was available");
            return Ok(None);
        }
        let deadline = Instant::now() + timeout;
        let queued_index = loop {
            if let Some(index) = self.queued_bulk_reads.iter().position(|queued| {
                queued.buffer.len() == byte_len && queued.complete.load(Ordering::Acquire)
            }) {
                break index;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(UsbTransferError::Timeout);
            }
            self.context
                .handle_events(Some(remaining))
                .map_err(native_usb_error)?;
        };
        let queued = self
            .queued_bulk_reads
            .remove(queued_index)
            .expect("queued USB receive exists");
        // SAFETY: completion was observed, so libusb no longer accesses this
        // request or its buffer.
        let (status, actual_length) =
            unsafe { ((*queued.transfer).status, (*queued.transfer).actual_length) };
        unsafe { rusb::ffi::libusb_free_transfer(queued.transfer) };
        if status != rusb::constants::LIBUSB_TRANSFER_COMPLETED || actual_length < 0 {
            return Err(if status == rusb::constants::LIBUSB_TRANSFER_TIMED_OUT {
                UsbTransferError::Timeout
            } else {
                UsbTransferError::Other
            });
        }
        let actual_length = usize::try_from(actual_length).map_err(|_| UsbTransferError::Other)?;
        if actual_length > queued.buffer.len() {
            return Err(UsbTransferError::Other);
        }
        let mut buffer = queued.buffer.into_vec();
        buffer.truncate(actual_length);
        Ok(Some(buffer))
    }

    /// Cancels and removes every queued bulk read.
    pub fn cancel_queued_bulk_read(&mut self) -> Result<(), UsbTransferError> {
        while let Some(queued) = self.queued_bulk_reads.pop_front() {
            if !queued.complete.load(Ordering::Acquire) {
                // SAFETY: this transport is the sole owner of the request.
                if unsafe { rusb::ffi::libusb_cancel_transfer(queued.transfer) } != 0 {
                    // libusb can still access the request after a failed cancel.
                    std::mem::forget(queued);
                    return Err(UsbTransferError::Other);
                }
                let deadline = Instant::now() + USB_CANCELLATION_TIMEOUT;
                while !queued.complete.load(Ordering::Acquire) {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        // Do not free memory that libusb may still access.
                        std::mem::forget(queued);
                        return Err(UsbTransferError::Timeout);
                    }
                    if self.context.handle_events(Some(remaining)).is_err() {
                        std::mem::forget(queued);
                        return Err(UsbTransferError::Other);
                    }
                }
            }
            unsafe { rusb::ffi::libusb_free_transfer(queued.transfer) };
        }
        Ok(())
    }

    /// Cancels pending reads and releases the claimed interface.
    pub fn close(&mut self) -> Result<(), UsbTransferError> {
        self.cancel_queued_bulk_read()?;
        if self.claimed {
            self.handle
                .release_interface(self.interface)
                .map_err(native_usb_error)?;
            self.claimed = false;
        }
        Ok(())
    }
}

impl Drop for NativeUsbDevice {
    fn drop(&mut self) {
        let _ = self.cancel_queued_bulk_read();
    }
}

fn native_usb_error(error: rusb::Error) -> UsbTransferError {
    if error == rusb::Error::Timeout {
        UsbTransferError::Timeout
    } else {
        UsbTransferError::Other
    }
}
