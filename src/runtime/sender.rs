//! Broadcast sender with watchdog monitoring for deadlock detection

use crossbeam_channel::{SendError, Sender as CrossbeamSender};

use super::watchdog::{OperationGuard, WatchdogHandle};

/// Broadcast sender that sends to one or more consumers
///
/// Architecture: Direct broadcast from caller thread to all destinations.
/// For head-of-line blocking prevention, use `split_senders()` to get
/// individual senders and spawn one thread per destination in your node.
///
/// Includes watchdog monitoring to detect blocked sends.
pub struct Sender<T> {
    destinations: Vec<CrossbeamSender<T>>,
    watchdog_handle: Option<WatchdogHandle>,
}

impl<T: Clone> Sender<T> {
    /// Create a new Sender from a vector of crossbeam senders
    pub fn new(destinations: Vec<CrossbeamSender<T>>) -> Self {
        Self {
            destinations,
            watchdog_handle: None,
        }
    }

    /// Attach a watchdog handle to monitor send operations
    pub fn with_watchdog(&self, watchdog_handle: WatchdogHandle) -> Self {
        Self {
            destinations: self.destinations.clone(),
            watchdog_handle: Some(watchdog_handle),
        }
    }

    /// Split this broadcast sender into individual senders (one per destination)
    ///
    /// This allows nodes to spawn one thread per destination rather than
    /// broadcasting from a single thread. Each returned Sender sends to
    /// exactly one destination.
    ///
    /// For nodes that need per-destination parallelism (like DslFileSource),
    /// use this to get independent senders and spawn your own threads.
    ///
    /// # Example
    /// ```ignore
    /// // In a self-threading node's work() method:
    /// let senders = output_port.split_senders::<MyType>()?;
    /// for (idx, sender) in senders.into_iter().enumerate() {
    ///     thread::spawn(move || {
    ///         // Each thread sends to one destination independently
    ///         sender.send(data)?;
    ///     });
    /// }
    /// ```
    pub fn split_senders(&self) -> Vec<Sender<T>> {
        self.destinations
            .iter()
            .map(|dest| Sender {
                destinations: vec![dest.clone()],
                watchdog_handle: self.watchdog_handle.clone(),
            })
            .collect()
    }

    /// Get the number of broadcast destinations
    pub fn num_destinations(&self) -> usize {
        self.destinations.len()
    }

    /// Send a value to all destinations
    ///
    /// Sends to all destinations sequentially with watchdog monitoring.
    /// If any destination blocks, the watchdog can detect it.
    ///
    /// For nodes that need non-blocking broadcast, use `split_senders()`
    /// to spawn one thread per destination.
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        if self.destinations.is_empty() {
            return Ok(());
        }

        // Create watchdog guard if watchdog is attached
        let _guard = self.watchdog_handle.as_ref().map(OperationGuard::new);

        // Send to all destinations
        let mut any_success = false;
        let mut last_error = None;

        for dest in &self.destinations {
            match dest.send(value.clone()) {
                Ok(()) => any_success = true,
                Err(e) => last_error = Some(e),
            }
        }

        // Only fail if no destination succeeded
        if !any_success && let Some(e) = last_error {
            return Err(e);
        }

        Ok(())
    }

    /// Try to send without blocking
    pub fn try_send(&self, value: T) -> Result<(), crossbeam_channel::TrySendError<T>> {
        if self.destinations.is_empty() {
            return Ok(());
        }

        for dest in &self.destinations {
            dest.try_send(value.clone())?;
        }

        Ok(())
    }

    /// Check if this sender has any connected receivers
    pub fn is_connected(&self) -> bool {
        !self.destinations.is_empty()
    }
}

impl<T: Clone> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            destinations: self.destinations.clone(),
            watchdog_handle: self.watchdog_handle.clone(),
        }
    }
}
