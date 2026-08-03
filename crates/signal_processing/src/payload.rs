//! Stable identities for payload types retained as derived data.
//!
//! The registry records durable payload identity and its typed ingestion
//! factory. Graph-level consumers decide separately whether a registered
//! payload is viewable, so collection and presentation remain independently
//! extensible.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use crate::derived_data_collector::{DerivedDataRetention, DerivedLanes};
use crate::derived_word_store::LiveStoreConfig;
use crate::errors::WorkResult;
use crate::events::WordPayload;
use crate::ports::{InputPort, PortSchema};

/// One type-erased collector input owned by a registered payload adapter.
///
/// Implementations downcast the input only to their registered payload type,
/// retain bounded data in their own storage, and publish an opaque query
/// handle through the [`CollectedLaneRequest`].
pub trait CollectedLaneIngestor: Send {
    /// Returns the port schema expected at a collector input position.
    ///
    /// # Parameters
    /// - `index`: Input position requested by the generic collector.
    fn input_schema(&self, index: usize) -> PortSchema;
    /// Drains available input values into adapter-owned retained storage.
    ///
    /// # Parameters
    /// - `input`: Typed generic input port to drain.
    /// - `retention`: Retention policy selected for the current run.
    fn drain(&mut self, input: &InputPort, retention: DerivedDataRetention) -> WorkResult<usize>;
    /// Returns whether the ingestor will accept no further values.
    fn is_finished(&self) -> bool;
}

/// Bounded visible-window request supplied to an adapter-owned retained query.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct CollectedLaneSnapshotRequest {
    /// Inclusive visible-window start in the shared timeline.
    pub start_time_ns: u64,
    /// Inclusive visible-window end in the shared timeline.
    pub end_time_ns: u64,
    /// Maximum snapshot items requested by the consumer.
    pub max_items: usize,
}

/// Revision and cardinality of one optional tabular lane projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollectedLaneTableMetadata {
    /// Revision used to invalidate table snapshots.
    pub generation: u64,
    /// Total rows retained by the adapter.
    pub total_rows: u64,
}

/// One scalar record supplied by an optional tabular lane projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectedLaneTableRow {
    /// Inclusive start timestamp of the record.
    pub start_time_ns: u64,
    /// Inclusive end timestamp of the record.
    pub end_time_ns: u64,
    /// Scalar value associated with the record.
    pub value: u64,
    /// Optional typed word payload retained with the scalar value.
    pub payload: Option<WordPayload>,
}

/// Bounded rows for an optional scalar table projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectedLaneTableSnapshot {
    /// Bounded rows returned by the query.
    pub rows: Vec<CollectedLaneTableRow>,
    /// Whether no more rows remain beyond this snapshot.
    pub complete: bool,
    /// Optional owner-defined scalar formatting hint.
    pub format_hint: Option<String>,
}

/// Physical retention used by one collected signal lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectedLaneStorageBacking {
    /// Exact data and its timeline summaries are retained in process memory.
    Memory,
    /// Exact data is held by an indexed store owned by the current run.
    Indexed,
    /// An immutable persistent cache entry was reopened for querying.
    PersistentCache,
    /// A plugin owns the storage and has not published further diagnostics.
    AdapterManaged,
}

/// Presentation-neutral storage diagnostics for one collected signal lane.
///
/// Byte counts are estimates of owned payload storage. They intentionally do
/// not include allocator bookkeeping or shared query-handle allocations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollectedLaneStorageSnapshot {
    /// Physical backing used by the lane.
    pub backing: CollectedLaneStorageBacking,
    /// Number of retained records, when measurable.
    pub retained_items: Option<u64>,
    /// In-memory bytes held by the lane, when measurable.
    pub resident_bytes: Option<u64>,
    /// Durable bytes held by the lane, when measurable.
    pub stored_bytes: Option<u64>,
    /// Number of index entries, when measurable.
    pub index_items: Option<u64>,
    /// Bytes occupied by indexes, when measurable.
    pub index_bytes: Option<u64>,
    /// Whether retained data can change without replacing the query.
    pub live: bool,
}

impl CollectedLaneStorageSnapshot {
    /// Creates minimal diagnostics for an adapter-managed lane.
    pub fn adapter_managed(live: bool) -> Self {
        Self {
            backing: CollectedLaneStorageBacking::AdapterManaged,
            retained_items: None,
            resident_bytes: None,
            stored_bytes: None,
            index_items: None,
            index_bytes: None,
            live,
        }
    }
}

/// Type-erased immutable result of a bounded retained-data query.
#[derive(Clone)]
pub struct OpaqueCollectedLaneSnapshot {
    value: Arc<dyn Any + Send + Sync>,
}

impl OpaqueCollectedLaneSnapshot {
    /// Erases one immutable adapter-owned snapshot value.
    ///
    /// # Parameters
    /// - `value`: Typed immutable snapshot to make available to consumers.
    pub fn new<T: Send + Sync + 'static>(value: Arc<T>) -> Self {
        Self { value }
    }

    /// Downcasts the snapshot to the adapter's registered result type.
    pub fn value<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        Arc::downcast::<T>(Arc::clone(&self.value)).ok()
    }
}

impl std::fmt::Debug for OpaqueCollectedLaneSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpaqueCollectedLaneSnapshot")
            .finish_non_exhaustive()
    }
}

/// Type-erased, adapter-owned retained-data query.
///
/// A collector publishes this after it has created the lane's storage. Data
/// subscribers may attach during or after a run and downcast it only to the
/// query type registered by that payload owner. The generic collector and
/// storage registry never inspect the concrete query value.
pub trait CollectedLaneQuery: Send + Sync {
    /// Erases the query for storage in the generic derived-lane catalog.
    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;

    /// Returns the revision of data observable through [`Self::snapshot`].
    ///
    /// Append-only and rolling adapters should change this value whenever a
    /// previously returned snapshot may have changed. Returning `None`
    /// disables subscriber-side snapshot caching for adapters without a
    /// revision contract.
    fn snapshot_generation(&self) -> Option<u64> {
        None
    }

    /// Produces an immutable, bounded snapshot for a visible window. The
    /// default declares that this query is panel-only and has no waveform
    /// representation.
    fn snapshot(
        &self,
        _request: CollectedLaneSnapshotRequest,
    ) -> Option<OpaqueCollectedLaneSnapshot> {
        None
    }

    /// Returns a nearby semantic time boundary for cursor snapping. The
    /// adapter defines which boundaries are meaningful for its payload; a
    /// query that has no snapping behavior returns `None`.
    fn nearest_time_boundary(&self, _timestamp_ns: u64, _max_distance_ns: u64) -> Option<u64> {
        None
    }

    /// Returns the greatest timeline timestamp retained by this lane. The
    /// adapter owns the exact span semantics for its payload; a query without
    /// timeline data returns `None`.
    fn timeline_extent_end_ns(&self) -> Option<u64> {
        None
    }

    /// Whether retained data can still change without replacing this query.
    fn is_live(&self) -> bool {
        false
    }

    /// Supplies revision metadata for a row-oriented scalar table. Queries
    /// without a table projection return `None`.
    fn table_metadata(&self) -> Option<CollectedLaneTableMetadata> {
        None
    }

    /// Supplies at most `max_rows` scalar table rows from the beginning of
    /// the retained sequence. `complete` reports whether more rows exist.
    fn table_snapshot(&self, _max_rows: usize) -> Option<CollectedLaneTableSnapshot> {
        None
    }

    /// Describes the retained data and indexes owned by this adapter.
    ///
    /// Plugin queries remain visible in diagnostics even when they do not
    /// opt into detailed accounting.
    fn storage_snapshot(&self) -> CollectedLaneStorageSnapshot {
        CollectedLaneStorageSnapshot::adapter_managed(self.is_live())
    }
}

/// Context supplied when a payload adapter creates one retained lane.
#[derive(Clone)]
pub struct CollectedLaneRequest {
    name: String,
    input_index: usize,
    lanes: DerivedLanes,
    payload: PayloadDescriptor,
    retention: DerivedDataRetention,
    indexed_store: Option<LiveStoreConfig>,
    options: Arc<dyn Any + Send + Sync>,
}

impl CollectedLaneRequest {
    /// Creates context for one adapter-owned retained output lane.
    ///
    /// # Parameters
    /// - `name`: Stable runtime lane name.
    /// - `input_index`: Collector input position producing the lane.
    /// - `lanes`: Run-owned catalog in which the query will be published.
    /// - `payload`: Stable identity of the collected payload type.
    /// - `retention`: Retention policy selected for the run.
    pub fn new(
        name: impl Into<String>,
        input_index: usize,
        lanes: DerivedLanes,
        payload: PayloadDescriptor,
        retention: DerivedDataRetention,
    ) -> Self {
        Self {
            name: name.into(),
            input_index,
            lanes,
            payload,
            retention,
            indexed_store: None,
            options: Arc::new(()),
        }
    }

    /// Returns the lane's stable runtime name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the collector input position producing the lane.
    pub fn input_index(&self) -> usize {
        self.input_index
    }

    /// Returns the run-owned derived-lane catalog.
    pub fn lanes(&self) -> &DerivedLanes {
        &self.lanes
    }

    /// Returns the stable payload descriptor.
    pub fn payload(&self) -> &PayloadDescriptor {
        &self.payload
    }

    /// Returns the lane retention policy.
    pub fn retention(&self) -> DerivedDataRetention {
        self.retention
    }

    /// Supplies the generic indexed storage selected for this collected lane.
    /// Payload adapters decide how their values map onto that storage format.
    pub fn with_indexed_store(mut self, config: LiveStoreConfig) -> Self {
        self.indexed_store = Some(config);
        self
    }

    /// Returns indexed storage selected by the compiler, when any.
    pub fn indexed_store(&self) -> Option<&LiveStoreConfig> {
        self.indexed_store.as_ref()
    }

    /// Attaches adapter-owned construction options without making the
    /// collector understand their concrete type.
    pub fn with_options<T: Send + Sync + 'static>(mut self, options: T) -> Self {
        self.options = Arc::new(options);
        self
    }

    /// Downcasts adapter-owned construction options to their declared type.
    pub fn options<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.options.downcast_ref::<T>()
    }

    /// Publishes an adapter-owned retained query under this lane's stable
    /// identity. Subscribers may resolve it immediately or after the
    /// producing run has finished.
    pub fn publish_query<T: CollectedLaneQuery + 'static>(&self, query: Arc<T>) {
        self.lanes
            .publish_opaque_lane(&self.name, self.payload.clone(), query);
    }
}

/// Factory for the typed ingestion and retained-query behavior of one payload.
pub trait PayloadAdapter: Send + Sync {
    /// Creates the typed ingestor and publishes its retained-query behavior.
    ///
    /// # Parameters
    /// - `request`: Generic lane identity, retention, storage, and opaque options.
    fn create_ingestor(
        &self,
        request: CollectedLaneRequest,
    ) -> Result<Box<dyn CollectedLaneIngestor>, String>;
}

/// Persistable identity assigned by a payload owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayloadDescriptor {
    stable_id: String,
}

impl PayloadDescriptor {
    /// Stable plugin-owned identity, suitable for saved state and diagnostics.
    pub fn stable_id(&self) -> &str {
        &self.stable_id
    }
}

/// Failure to add an ambiguous payload identity.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PayloadRegistrationError {
    #[error("payload identifiers must not be empty")]
    EmptyStableId,
    #[error(
        "payload type is already registered as '{existing_stable_id}', not '{requested_stable_id}'"
    )]
    TypeAlreadyRegistered {
        existing_stable_id: String,
        requested_stable_id: String,
    },
    #[error("payload identifier '{stable_id}' is already registered for another type")]
    StableIdAlreadyRegistered { stable_id: String },
    #[error("payload '{stable_id}' already has an ingestion adapter")]
    AdapterAlreadyRegistered { stable_id: String },
    #[error("payload type '{type_name}' has no payload identity")]
    PayloadNotRegistered { type_name: String },
    #[error("payload '{stable_id}' has no ingestion adapter")]
    PayloadHasNoAdapter { stable_id: String },
}

/// Bidirectional identity registry for payload types.
///
/// `TypeId` selects a typed channel while the application runs. `stable_id`
/// is the durable identity for serialized graph and panel state. Registering
/// the same type and identifier is idempotent; every other collision fails.
#[derive(Clone, Default)]
pub struct PayloadRegistry {
    by_type: HashMap<TypeId, PayloadDescriptor>,
    by_stable_id: HashMap<String, TypeId>,
    adapters: HashMap<TypeId, Arc<dyn PayloadAdapter>>,
}

impl std::fmt::Debug for PayloadRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PayloadRegistry")
            .field("by_type", &self.by_type)
            .field("adapter_count", &self.adapters.len())
            .finish()
    }
}

impl PayloadRegistry {
    /// Creates an empty payload identity and adapter registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a stable identity for a concrete payload Rust type.
    pub fn register<T: Clone + Send + Sync + 'static>(
        &mut self,
        stable_id: impl Into<String>,
    ) -> Result<(), PayloadRegistrationError> {
        let stable_id = stable_id.into();
        if stable_id.trim().is_empty() {
            return Err(PayloadRegistrationError::EmptyStableId);
        }

        let type_id = TypeId::of::<T>();
        if let Some(existing) = self.by_type.get(&type_id) {
            return if existing.stable_id == stable_id {
                Ok(())
            } else {
                Err(PayloadRegistrationError::TypeAlreadyRegistered {
                    existing_stable_id: existing.stable_id.clone(),
                    requested_stable_id: stable_id,
                })
            };
        }
        if self.by_stable_id.contains_key(&stable_id) {
            return Err(PayloadRegistrationError::StableIdAlreadyRegistered { stable_id });
        }

        self.by_stable_id.insert(stable_id.clone(), type_id);
        self.by_type
            .insert(type_id, PayloadDescriptor { stable_id });
        Ok(())
    }

    /// Registers a payload identity supplied through a compile-time plugin descriptor.
    ///
    /// # Parameters
    /// - `type_id`: Concrete payload type identity supplied by a plugin.
    /// - `stable_id`: Stable persisted payload identity.
    pub fn register_erased(
        &mut self,
        type_id: TypeId,
        stable_id: impl Into<String>,
    ) -> Result<(), PayloadRegistrationError> {
        let stable_id = stable_id.into();
        if stable_id.trim().is_empty() {
            return Err(PayloadRegistrationError::EmptyStableId);
        }
        if let Some(existing) = self.by_type.get(&type_id) {
            return if existing.stable_id == stable_id {
                Ok(())
            } else {
                Err(PayloadRegistrationError::TypeAlreadyRegistered {
                    existing_stable_id: existing.stable_id.clone(),
                    requested_stable_id: stable_id,
                })
            };
        }
        if self.by_stable_id.contains_key(&stable_id) {
            return Err(PayloadRegistrationError::StableIdAlreadyRegistered { stable_id });
        }
        self.by_stable_id.insert(stable_id.clone(), type_id);
        self.by_type
            .insert(type_id, PayloadDescriptor { stable_id });
        Ok(())
    }

    /// Returns the descriptor registered for a concrete payload type.
    pub fn descriptor<T: 'static>(&self) -> Option<&PayloadDescriptor> {
        self.descriptor_by_type_id(TypeId::of::<T>())
    }

    /// Returns the descriptor registered for a type identity.
    pub fn descriptor_by_type_id(&self, type_id: TypeId) -> Option<&PayloadDescriptor> {
        self.by_type.get(&type_id)
    }

    /// Returns the descriptor registered for a stable persisted identity.
    pub fn descriptor_by_stable_id(&self, stable_id: &str) -> Option<&PayloadDescriptor> {
        self.by_stable_id
            .get(stable_id)
            .and_then(|type_id| self.descriptor_by_type_id(*type_id))
    }

    /// Adds the typed ingestion factory for an already identified payload.
    pub fn register_adapter<T: Clone + Send + Sync + 'static>(
        &mut self,
        adapter: Arc<dyn PayloadAdapter>,
    ) -> Result<(), PayloadRegistrationError> {
        let type_id = TypeId::of::<T>();
        let Some(descriptor) = self.by_type.get(&type_id) else {
            return Err(PayloadRegistrationError::PayloadNotRegistered {
                type_name: std::any::type_name::<T>().to_owned(),
            });
        };
        if self.adapters.contains_key(&type_id) {
            return Err(PayloadRegistrationError::AdapterAlreadyRegistered {
                stable_id: descriptor.stable_id.clone(),
            });
        }
        self.adapters.insert(type_id, adapter);
        Ok(())
    }

    /// Adds a type-erased adapter supplied through a compile-time plugin descriptor.
    pub fn register_adapter_erased(
        &mut self,
        type_id: TypeId,
        type_name: &str,
        adapter: Arc<dyn PayloadAdapter>,
    ) -> Result<(), PayloadRegistrationError> {
        let Some(descriptor) = self.by_type.get(&type_id) else {
            return Err(PayloadRegistrationError::PayloadNotRegistered {
                type_name: type_name.to_owned(),
            });
        };
        if self.adapters.contains_key(&type_id) {
            return Err(PayloadRegistrationError::AdapterAlreadyRegistered {
                stable_id: descriptor.stable_id.clone(),
            });
        }
        self.adapters.insert(type_id, adapter);
        Ok(())
    }

    /// Returns the ingestion adapter registered for a type identity.
    ///
    /// # Parameters
    /// - `type_id`: Concrete payload type identity to look up.
    pub fn adapter_by_type_id(&self, type_id: TypeId) -> Option<&Arc<dyn PayloadAdapter>> {
        self.adapters.get(&type_id)
    }
}

#[cfg(test)]
mod payload_tests {
    use super::*;

    #[derive(Clone)]
    struct First;
    #[derive(Clone)]
    struct Second;

    struct TestQuery(Vec<u64>);

    impl CollectedLaneQuery for TestQuery {
        fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
            self
        }

        fn snapshot(
            &self,
            request: CollectedLaneSnapshotRequest,
        ) -> Option<OpaqueCollectedLaneSnapshot> {
            Some(OpaqueCollectedLaneSnapshot::new(Arc::new(
                self.0
                    .iter()
                    .copied()
                    .take(request.max_items)
                    .collect::<Vec<_>>(),
            )))
        }
    }

    struct FailingAdapter;

    impl PayloadAdapter for FailingAdapter {
        fn create_ingestor(
            &self,
            _request: CollectedLaneRequest,
        ) -> Result<Box<dyn CollectedLaneIngestor>, String> {
            Err("not used by registration test".to_owned())
        }
    }

    #[test]
    fn same_type_and_stable_id_is_idempotent() {
        let mut registry = PayloadRegistry::new();

        registry.register::<First>("org.example.first/v1").unwrap();
        registry.register::<First>("org.example.first/v1").unwrap();

        assert_eq!(
            registry.descriptor::<First>().unwrap().stable_id(),
            "org.example.first/v1"
        );
    }

    #[test]
    fn erased_plugin_registration_preserves_type_identity_and_adapter() {
        let mut registry = PayloadRegistry::new();
        let type_id = TypeId::of::<First>();

        registry
            .register_erased(type_id, "org.example.first/v1")
            .unwrap();
        registry
            .register_adapter_erased(type_id, "First", Arc::new(FailingAdapter))
            .unwrap();

        assert_eq!(
            registry.descriptor_by_type_id(type_id).unwrap().stable_id(),
            "org.example.first/v1"
        );
        assert!(registry.adapter_by_type_id(type_id).is_some());
    }

    #[test]
    fn rejects_type_or_stable_id_collisions() {
        let mut registry = PayloadRegistry::new();
        registry.register::<First>("org.example.first/v1").unwrap();

        assert!(matches!(
            registry.register::<First>("org.example.renamed/v1"),
            Err(PayloadRegistrationError::TypeAlreadyRegistered { .. })
        ));
        assert!(matches!(
            registry.register::<Second>("org.example.first/v1"),
            Err(PayloadRegistrationError::StableIdAlreadyRegistered { .. })
        ));
    }

    #[test]
    fn registered_identity_accepts_one_typed_ingestion_adapter() {
        let mut registry = PayloadRegistry::new();
        registry.register::<First>("org.example.first/v1").unwrap();

        registry
            .register_adapter::<First>(Arc::new(FailingAdapter))
            .unwrap();

        assert!(registry.adapter_by_type_id(TypeId::of::<First>()).is_some());
        assert!(matches!(
            registry.register_adapter::<First>(Arc::new(FailingAdapter)),
            Err(PayloadRegistrationError::AdapterAlreadyRegistered { .. })
        ));
    }

    #[test]
    fn adapter_registration_requires_a_payload_identity() {
        let mut registry = PayloadRegistry::new();

        assert!(matches!(
            registry.register_adapter::<First>(Arc::new(FailingAdapter)),
            Err(PayloadRegistrationError::PayloadNotRegistered { .. })
        ));
    }

    #[test]
    fn request_publishes_an_adapter_owned_query() {
        let lanes = DerivedLanes::new();
        let mut registry = PayloadRegistry::new();
        registry.register::<First>("org.example.first/v1").unwrap();
        let request = CollectedLaneRequest::new(
            "first",
            0,
            lanes.clone(),
            registry.descriptor::<First>().unwrap().clone(),
            DerivedDataRetention::Unlimited,
        );

        request.publish_query(Arc::new(TestQuery(vec![1_u64, 2, 3])));

        let snapshot = lanes.opaque_lanes()[0]
            .snapshot(CollectedLaneSnapshotRequest {
                start_time_ns: 0,
                end_time_ns: 1,
                max_items: 2,
            })
            .unwrap();
        let values = snapshot.value::<Vec<u64>>().unwrap();
        assert_eq!(values.as_slice(), &[1, 2]);
        assert_eq!(
            lanes.opaque_lanes()[0].storage_snapshot(),
            CollectedLaneStorageSnapshot::adapter_managed(false)
        );
    }
}
