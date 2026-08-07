//! Type registry for dynamic channel creation.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crossbeam_channel::{Receiver as CrossbeamReceiver, Sender as CrossbeamSender, bounded};

use super::errors::PipelineError;
use super::sender::{ChannelMessage, OverflowPolicy, Sender, SharedSenders};

/// Type-erased view of a [`SharedSenders<T>`] so the `PipelineManager` can
/// own and rewire subscriber lists without knowing `T`.
pub(crate) trait ErasedSharedSenders: Send + Sync {
    fn subscribe_with_label(
        &self,
        buffer: usize,
        policy: OverflowPolicy,
        label: Option<String>,
    ) -> ErasedSubscription;
    fn unsubscribe(&self, id: u64);
    /// EOS to all subscribers; late joiners get an immediate EOS.
    fn close(&self);
    /// A boxed `Sender<T>` broadcasting through this list, for
    /// `OutputPort::from_type_erased`.
    fn sender_box(&self) -> Box<dyn Any + Send>;
    /// Subscription ids dropped by `OverflowPolicy::Disconnect`.
    fn take_disconnected(&self) -> Vec<u64>;
    /// Whether broadcasting the next value would have to block on some
    /// subscriber — see [`SharedSenders::would_block`].
    fn would_block(&self) -> bool;
}

/// Type-erased input readiness used by the single-threaded runner without
/// constraining channels to a closed set of payload types.
pub(crate) trait ErasedReceiverReadiness: Send {
    fn is_empty(&self) -> bool;
}

/// A typed receiver accompanied by the type-erased readiness view needed by
/// the cooperative scheduler.
pub(crate) struct ErasedSubscription {
    pub(crate) id: u64,
    pub(crate) receiver: Box<dyn Any + Send>,
    pub(crate) readiness: Box<dyn ErasedReceiverReadiness>,
}

struct ReceiverReadiness<T> {
    receiver: CrossbeamReceiver<ChannelMessage<T>>,
}

impl<T: Send + 'static> ErasedReceiverReadiness for ReceiverReadiness<T> {
    fn is_empty(&self) -> bool {
        self.receiver.is_empty()
    }
}

impl<T: Clone + Send + Sync + 'static> ErasedSharedSenders for SharedSenders<T> {
    fn subscribe_with_label(
        &self,
        buffer: usize,
        policy: OverflowPolicy,
        label: Option<String>,
    ) -> ErasedSubscription {
        let (id, rx) = SharedSenders::subscribe_with_label(self, buffer, policy, label);
        ErasedSubscription {
            id,
            receiver: Box::new(rx.clone()),
            readiness: Box::new(ReceiverReadiness { receiver: rx }),
        }
    }
    fn unsubscribe(&self, id: u64) {
        SharedSenders::unsubscribe(self, id);
    }
    fn close(&self) {
        SharedSenders::close(self);
    }
    fn sender_box(&self) -> Box<dyn Any + Send> {
        Box::new(Sender::from_shared(self.clone())) as Box<dyn Any + Send>
    }
    fn take_disconnected(&self) -> Vec<u64> {
        SharedSenders::take_disconnected(self)
    }
    fn would_block(&self) -> bool {
        SharedSenders::would_block(self)
    }
}

/// Type registry for creating channels dynamically based on TypeId
type ChannelCreatorFn =
    Box<dyn Fn(usize) -> (Box<dyn Any + Send>, Box<dyn Any + Send>) + Send + Sync>;
type OutputWrapperFn = Box<
    dyn Fn(Vec<Box<dyn Any + Send>>) -> Result<Box<dyn Any + Send>, PipelineError> + Send + Sync,
>;
type SharedCreatorFn = Box<dyn Fn(bool) -> Arc<dyn ErasedSharedSenders> + Send + Sync>;

pub(crate) struct LabeledSenderBox {
    pub sender: Box<dyn Any + Send>,
    pub label: Option<String>,
}

pub(crate) struct TypeRegistry {
    channel_creators: HashMap<TypeId, ChannelCreatorFn>,
    output_wrappers: HashMap<TypeId, OutputWrapperFn>,
    shared_creators: HashMap<TypeId, SharedCreatorFn>,
}

impl TypeRegistry {
    fn new() -> Self {
        Self {
            channel_creators: HashMap::new(),
            output_wrappers: HashMap::new(),
            shared_creators: HashMap::new(),
        }
    }

    /// Register a type for use in channels
    fn register<T: 'static + Send + Sync + Clone>(&mut self) {
        let type_id = TypeId::of::<T>();

        self.shared_creators.insert(
            type_id,
            Box::new(|sticky: bool| {
                Arc::new(SharedSenders::<T>::new(sticky)) as Arc<dyn ErasedSharedSenders>
            }),
        );

        // Register channel creator — channels carry ChannelMessage<T> internally
        self.channel_creators.insert(
            type_id,
            Box::new(|buffer_size: usize| {
                let (tx, rx) = bounded::<ChannelMessage<T>>(buffer_size);
                (
                    Box::new(tx) as Box<dyn Any + Send>,
                    Box::new(rx) as Box<dyn Any + Send>,
                )
            }),
        );

        // Register output wrapper
        self.output_wrappers.insert(
            type_id,
            Box::new(|senders: Vec<Box<dyn Any + Send>>| {
                if senders.is_empty() {
                    return Err(PipelineError::OutputChannelUnavailable);
                }

                let mut typed_senders = Vec::new();
                for sender in senders {
                    match sender.downcast::<LabeledSenderBox>() {
                        Ok(labeled) => {
                            let LabeledSenderBox { sender, label } = *labeled;
                            match sender.downcast::<CrossbeamSender<ChannelMessage<T>>>() {
                                Ok(tx) => typed_senders.push((*tx, label)),
                                Err(_) => return Err(PipelineError::OutputChannelTypeMismatch),
                            }
                        }
                        Err(sender) => {
                            match sender.downcast::<CrossbeamSender<ChannelMessage<T>>>() {
                                Ok(tx) => typed_senders.push((*tx, None)),
                                Err(_) => return Err(PipelineError::OutputChannelTypeMismatch),
                            }
                        }
                    }
                }

                // Create Sender without watchdog (will be attached by OutputPort)
                let broadcast_sender = Sender::new_labeled(typed_senders);

                Ok(Box::new(broadcast_sender) as Box<dyn Any + Send>)
            }),
        );
    }

    pub(crate) fn create_channel(
        &self,
        type_id: TypeId,
        buffer_size: usize,
    ) -> Option<(Box<dyn Any + Send>, Box<dyn Any + Send>)> {
        self.channel_creators
            .get(&type_id)
            .map(|creator| creator(buffer_size))
    }

    pub(crate) fn wrap_output(
        &self,
        type_id: TypeId,
        senders: Vec<Box<dyn Any + Send>>,
    ) -> Result<Box<dyn Any + Send>, PipelineError> {
        self.output_wrappers
            .get(&type_id)
            .ok_or(PipelineError::TypeNotRegistered { type_id })?(senders)
    }

    /// Creates a supervisor-owned subscriber list for `type_id`.
    pub(crate) fn create_shared(
        &self,
        type_id: TypeId,
        sticky: bool,
    ) -> Option<Arc<dyn ErasedSharedSenders>> {
        self.shared_creators
            .get(&type_id)
            .map(|creator| creator(sticky))
    }
}

// Global type registry
lazy_static::lazy_static! {
    pub(crate) static ref TYPE_REGISTRY: Arc<Mutex<TypeRegistry>> = {
        Arc::new(Mutex::new(TypeRegistry::new()))
    };
}

/// Registers a custom cloneable payload type for graph channels.
///
/// Call this before building a pipeline or live manager that exposes `T` on a
/// port, so generic channel construction can recover its concrete type.
pub fn register_type<T: 'static + Send + Sync + Clone>() {
    TYPE_REGISTRY.lock().unwrap().register::<T>();
}
