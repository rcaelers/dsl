//! Channel receivers with per-channel putback buffers and watchdog monitoring
//!
//! - [`Receiver`] wraps a single `crossbeam_channel::Receiver<T>`
//!   with a putback buffer, providing `recv`, `peek`, `put_back`, and
//!   `drain_before` operations.
//!
//! - [`ReceiverSelector`] performs a multiplexed `select()` across
//!   a slice of `Receiver`s, checking buffers before blocking.

use crossbeam_channel::Receiver as CrossbeamReceiver;
use std::collections::VecDeque;

use super::errors::{WorkError, WorkResult};
use super::watchdog::{OperationGuard, WatchdogHandle};

// ────────────────────────────────────────────────────────────────────────────
// Receiver — single-channel wrapper
// ────────────────────────────────────────────────────────────────────────────

/// A single crossbeam receiver with a putback buffer.
///
/// The buffer is externally owned (passed as `&mut VecDeque<T>`) so it
/// persists across calls in the owning node's struct.
///
/// Includes watchdog monitoring for deadlock detection (zero-cost with atomics).
pub struct Receiver<'a, T> {
    receiver: &'a CrossbeamReceiver<T>,
    buffer: &'a mut VecDeque<T>,
    watchdog_handle: Option<WatchdogHandle>,
}

impl<'a, T> Receiver<'a, T> {
    /// Create a new receiver with watchdog monitoring.
    pub fn new(
        receiver: &'a CrossbeamReceiver<T>,
        buffer: &'a mut VecDeque<T>,
        watchdog_handle: WatchdogHandle,
    ) -> Self {
        Self {
            receiver,
            buffer,
            watchdog_handle: Some(watchdog_handle),
        }
    }

    /// Create a new receiver with watchdog monitoring.
    pub fn with_watchdog(
        receiver: &'a CrossbeamReceiver<T>,
        buffer: &'a mut VecDeque<T>,
        watchdog_handle: WatchdogHandle,
    ) -> Self {
        // Just call new() - they're the same now
        Self::new(receiver, buffer, watchdog_handle)
    }

    /// Blocking receive. Returns from the putback buffer first, then
    /// falls through to the underlying channel.
    pub fn recv(&mut self) -> WorkResult<T> {
        if let Some(item) = self.buffer.pop_front() {
            return Ok(item);
        }

        // Create watchdog guard if watchdog is attached (zero-cost: just 8-byte stack ref + 2 atomic stores)
        let _guard = self.watchdog_handle.as_ref().map(OperationGuard::new);
        self.receiver.recv().map_err(|_| {
            tracing::debug!("Receiver::recv() - channel disconnected, returning Shutdown");
            WorkError::Shutdown
        })
    }

    /// Peek at the front item. If the buffer is empty, blocks on `recv()`
    /// to populate it.
    pub fn peek(&mut self) -> WorkResult<&T> {
        if self.buffer.is_empty() {
            // Create watchdog guard if watchdog is attached (zero-cost: just 8-byte stack ref + 2 atomic stores)
            let _guard = self.watchdog_handle.as_ref().map(OperationGuard::new);
            let item = self.receiver.recv().map_err(|_| {
                tracing::debug!("Receiver::peek() - channel disconnected, returning Shutdown");
                WorkError::Shutdown
            })?;
            self.buffer.push_back(item);
        }
        Ok(self.buffer.front().unwrap())
    }

    /// Try to receive without blocking. Returns from the putback buffer first,
    /// then tries the underlying channel. Returns Err if would block or channel is closed.
    pub fn try_recv(&mut self) -> Result<T, crossbeam_channel::TryRecvError> {
        if let Some(item) = self.buffer.pop_front() {
            return Ok(item);
        }
        // No watchdog needed - this doesn't block
        self.receiver.try_recv()
    }

    /// Receive with a timeout. Returns from the putback buffer first (immediate),
    /// then tries the underlying channel with timeout.
    pub fn recv_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<T, crossbeam_channel::RecvTimeoutError> {
        if let Some(item) = self.buffer.pop_front() {
            return Ok(item);
        }
        // Watchdog guard for timeout recv if watchdog is attached (zero-cost: just 8-byte stack ref + 2 atomic stores)
        let _guard = self.watchdog_handle.as_ref().map(OperationGuard::new);
        self.receiver.recv_timeout(timeout)
    }

    /// Push an item back to the front of the buffer so the next `recv()`
    /// returns it.
    pub fn put_back(&mut self, item: T) {
        self.buffer.push_front(item);
    }

    /// Check if there are any buffered items.
    pub fn has_buffered(&self) -> bool {
        !self.buffer.is_empty()
    }

    /// Discard all items whose end time is `<= before`, blocking until the
    /// first item that extends past the threshold is buffered.
    ///
    /// With Sample format, an item is valid from `start_time` until the
    /// next item's `start_time`. So we need to look at pairs of items to
    /// determine if the first one ended before `before`.
    ///
    /// `start_time_fn` extracts the start time from each item.
    pub fn drain_before(
        &mut self,
        before: u64,
        start_time_fn: impl Fn(&T) -> u64,
    ) -> WorkResult<()> {
        loop {
            let current = self.recv()?;
            // Peek at next to see when current ends
            let next = self.peek()?;
            if start_time_fn(next) <= before {
                // Next starts at or before 'before', so current ended before 'before'
                // Discard current, continue
                continue;
            } else {
                // Next starts after 'before', so current extends past 'before'
                // Put current back - it's the first item we want to keep
                self.put_back(current);
                return Ok(());
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// ReceiverSelector — multiplexed select across channels
// ────────────────────────────────────────────────────────────────────────────

/// Multiplexed select across a slice of [`Receiver`]s.
///
/// Checks putback buffers before blocking on `crossbeam_channel::Select`.
/// Created transiently when a select is needed; individual channels are
/// used directly for single-channel operations.
pub struct ReceiverSelector<'b, 'a, T> {
    channels: &'b mut [Receiver<'a, T>],
}

impl<'b, 'a, T> ReceiverSelector<'b, 'a, T> {
    /// Create a selector over a mutable slice of receivers.
    pub fn new(channels: &'b mut [Receiver<'a, T>]) -> Self {
        Self { channels }
    }

    /// Number of channels.
    pub fn len(&self) -> usize {
        self.channels.len()
    }

    /// Whether the selector has no channels.
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Blocking receive from any channel. Checks all putback buffers first
    /// (in order), then falls through to `crossbeam_channel::Select`.
    ///
    /// Returns `(channel_index, item)`.
    pub fn select(&mut self) -> WorkResult<(usize, T)> {
        // Check buffers first (round-robin from index 0)
        for (i, ch) in self.channels.iter_mut().enumerate() {
            if let Some(item) = ch.buffer.pop_front() {
                return Ok((i, item));
            }
        }

        // All buffers empty — block on crossbeam Select
        let mut sel = crossbeam_channel::Select::new();
        for ch in self.channels.iter() {
            sel.recv(ch.receiver);
        }

        let oper = sel.select();
        let idx = oper.index();
        let item = oper
            .recv(self.channels[idx].receiver)
            .map_err(|_| {
                tracing::debug!("ReceiverSelector::select() - channel {} disconnected, returning Shutdown", idx);
                WorkError::Shutdown
            })?;
        Ok((idx, item))
    }

    /// Blocking select from a subset of channels by index. Checks buffers
    /// first, then blocks on `crossbeam_channel::Select` for only those
    /// channels.
    ///
    /// Returns `(channel_index, item)` where `channel_index` is the
    /// original index into the slice.
    pub fn select_from(&mut self, indices: &[usize]) -> WorkResult<(usize, T)> {
        // Check buffers first
        for &i in indices {
            if let Some(item) = self.channels[i].buffer.pop_front() {
                return Ok((i, item));
            }
        }

        // Block on crossbeam Select for specified channels only
        let mut sel = crossbeam_channel::Select::new();
        for &i in indices {
            sel.recv(self.channels[i].receiver);
        }

        let oper = sel.select();
        let sel_idx = oper.index(); // index within the select operation list
        let rx_idx = indices[sel_idx]; // map back to original channel index
        let item = oper
            .recv(self.channels[rx_idx].receiver)
            .map_err(|_| {
                tracing::debug!("ReceiverSelector::select_from() - channel {} disconnected, returning Shutdown", rx_idx);
                WorkError::Shutdown
            })?;
        Ok((rx_idx, item))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::bounded;

    // Helper to create a test watchdog
    fn test_watchdog() -> crate::runtime::Watchdog {
        crate::runtime::Watchdog::new()
    }

    // ── Receiver tests ───────────────────────────────────────────

    #[test]
    fn test_recv_from_buffer_then_channel() {
        let (tx, rx) = bounded::<i32>(10);
        let mut buf = VecDeque::new();
        buf.push_back(42);

        let wd = test_watchdog();
        let handle = wd.register_port("test", "recv", "test_port");
        let mut pr = Receiver::new(&rx, &mut buf, handle);

        // First recv comes from buffer
        assert_eq!(pr.recv().unwrap(), 42);

        // Second recv comes from channel
        tx.send(99).unwrap();
        assert_eq!(pr.recv().unwrap(), 99);

        drop(tx);
    }

    #[test]
    fn test_put_back_and_peek() {
        let (tx, rx) = bounded::<i32>(10);
        let mut buf = VecDeque::new();

        let wd = test_watchdog();
        let handle = wd.register_port("test", "recv", "test_port");
        let mut pr = Receiver::new(&rx, &mut buf, handle);

        assert!(!pr.has_buffered());

        pr.put_back(77);
        assert_eq!(pr.peek().unwrap(), &77);
        assert!(pr.has_buffered());

        assert_eq!(pr.recv().unwrap(), 77);
        assert!(!pr.has_buffered());
        
        drop(tx);
    }

    #[test]
    fn test_drain_before() {
        let (tx, rx) = bounded::<(u64, i32)>(10);
        let mut buf = VecDeque::new();

        // Add items to buffer (start_time, value)
        // These items extend: [100..200), [200..300), [300..inf)
        buf.push_back((100, 1));
        buf.push_back((200, 2));
        buf.push_back((300, 3));

        // Send more via channel
        tx.send((150, 4)).unwrap(); // Starts at 150, but we already have (100,1) and (200,2) in buffer
        tx.send((400, 5)).unwrap();

        let wd = test_watchdog();
        let handle = wd.register_port("test", "recv", "test_port");
        let mut pr = Receiver::new(&rx, &mut buf, handle);

        // Drain everything that ends at or before 200
        // With Sample format: item valid from start_time until next item's start_time
        // (100, 1) extends [100..200) - ends at 200, should be drained
        // (200, 2) extends [200..300) - ends at 300, should NOT be drained
        pr.drain_before(200, |item| item.0).unwrap();

        // Should have kept (200, 2) which ends at 300
        let val = pr.recv().unwrap();
        assert_eq!(val, (200, 2));

        drop(tx);
    }

    // ── ReceiverSelector tests ───────────────────────────────────

    #[test]
    fn test_select_from_buffers() {
        let (tx1, rx1) = bounded::<i32>(10);
        let (tx2, rx2) = bounded::<i32>(10);
        let mut buf0 = VecDeque::new();
        let mut buf1 = VecDeque::new();

        buf0.push_back(42);
        buf1.push_back(99);

        let wd = test_watchdog();
        let h0 = wd.register_port("test", "recv", "ch0");
        let h1 = wd.register_port("test", "recv", "ch1");
        let mut ch0 = Receiver::new(&rx1, &mut buf0, h0);
        let mut ch1 = Receiver::new(&rx2, &mut buf1, h1);

        {
            let mut sel = ReceiverSelector::new(std::slice::from_mut(&mut ch0));
            let (idx, val) = sel.select().unwrap();
            assert_eq!(idx, 0);
            assert_eq!(val, 42);
        }

        // Test using the second channel directly
        assert_eq!(ch1.recv().unwrap(), 99);

        // Now test select with channel data
        tx1.send(10).unwrap();
        let mut sel = ReceiverSelector::new(std::slice::from_mut(&mut ch0));
        let (idx, val) = sel.select().unwrap();
        assert_eq!(idx, 0);
        assert_eq!(val, 10);

        drop(tx1);
        drop(tx2);
    }

    #[test]
    fn test_select_multiple_channels() {
        let (tx1, rx1) = bounded::<i32>(10);
        let (tx2, rx2) = bounded::<i32>(10);
        let mut buf0 = VecDeque::new();
        let mut buf1 = VecDeque::new();

        // Use a Vec to get a contiguous slice
        let wd = test_watchdog();
        let h0 = wd.register_port("test", "recv", "ch0");
        let h1 = wd.register_port("test", "recv", "ch1");
        let mut channels = vec![
            Receiver::new(&rx1, &mut buf0, h0),
            Receiver::new(&rx2, &mut buf1, h1),
        ];

        // Put items in buffers via put_back
        channels[0].put_back(42);
        channels[1].put_back(99);

        let mut sel = ReceiverSelector::new(&mut channels);

        // Should read buffer 0 first
        let (idx, val) = sel.select().unwrap();
        assert_eq!(idx, 0);
        assert_eq!(val, 42);

        // Then buffer 1
        let (idx, val) = sel.select().unwrap();
        assert_eq!(idx, 1);
        assert_eq!(val, 99);

        // Now from channel
        tx1.send(10).unwrap();
        let (idx, val) = sel.select().unwrap();
        assert_eq!(idx, 0);
        assert_eq!(val, 10);

        drop(tx1);
        drop(tx2);
    }
}
