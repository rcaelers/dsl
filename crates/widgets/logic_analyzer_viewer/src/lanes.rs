use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard};

use egui::{Color32, Painter, Rect, Stroke};

use signal_processing::OpaqueCollectedLaneSnapshot;

/// Explicit identity of one payload in [`signal_processing::DerivedLanes`].
///
/// The current runtime store uses its lane name as its stable key. Wrapping
/// it prevents presentation code from treating that key as display text or
/// inferring behavior from it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DerivedLaneId(String);

impl DerivedLaneId {
    /// Creates a stable derived-lane identity from an owner-provided key.
    ///
    /// # Parameters
    /// - `value`: Stable lane key; it is not treated as display text.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the stable lane key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable presentation identity for a compound derived-lane group.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewerLaneGroupId(String);

impl ViewerLaneGroupId {
    /// Creates a lane-group identity from an owner-provided stable key.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the stable group key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable presentation identity for a track within a lane group.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewerLaneTrackId(String);

impl ViewerLaneTrackId {
    /// Creates a lane-track identity from an owner-provided stable key.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the stable track key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Short colored badge displayed beside a derived lane group.
#[derive(Debug, Clone)]
pub struct ViewerLaneBadge {
    /// Badge text.
    pub text: String,
    /// Badge color.
    pub color: Color32,
}

impl ViewerLaneBadge {
    /// Creates a lane badge with text and display color.
    pub fn new(text: impl Into<String>, color: Color32) -> Self {
        Self {
            text: text.into(),
            color,
        }
    }
}

/// One derived-data track rendered within a lane group.
#[derive(Debug, Clone)]
pub struct ViewerLaneTrack {
    /// Stable presentation identifier within the group.
    pub id: ViewerLaneTrackId,
    /// Stable derived-data lane supplying this track.
    pub lane: DerivedLaneId,
    /// Height multiplier relative to a base viewer row.
    pub relative_height: f32,
}

impl ViewerLaneTrack {
    /// Creates a track and clamps its height multiplier to a usable minimum.
    pub fn new(id: impl Into<String>, lane: DerivedLaneId, relative_height: f32) -> Self {
        Self {
            id: ViewerLaneTrackId::new(id),
            lane,
            relative_height: relative_height.max(0.25),
        }
    }
}

/// Fully resolved visual properties for one annotation box.
#[derive(Debug, Clone)]
pub struct AnnotationVisual {
    /// Text rendered inside the annotation.
    pub label: String,
    /// Annotation background color.
    pub fill: Color32,
    /// Annotation outline stroke.
    pub border: Stroke,
}

/// Geometry and drawing access supplied to an adapter-owned opaque payload
/// renderer. The painter is already clipped to the waveform region.
pub struct OpaqueLaneDrawContext<'a> {
    /// Painter clipped to the waveform region.
    pub painter: &'a Painter,
    /// Bounds of the waveform drawing region.
    pub wave_rect: Rect,
    /// Top coordinate of the lane group.
    pub top: f32,
    /// Lane group height in points.
    pub height: f32,
    /// First visible timeline timestamp.
    pub visible_start_ns: u64,
    /// Last visible timeline timestamp.
    pub visible_end_ns: u64,
    /// Viewer color roles for renderer-owned drawing.
    pub theme: ViewerLaneTheme,
    /// Bounded hover and cursor data for renderer interaction.
    pub interaction: ViewerLaneInteractionContext,
}

/// Theme roles available to payload renderers without exposing viewer state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewerLaneTheme {
    /// Lane background color.
    pub background: Color32,
    /// Primary text and stroke color.
    pub foreground: Color32,
    /// De-emphasized text and stroke color.
    pub muted_foreground: Color32,
    /// Highlight color.
    pub accent: Color32,
    /// Error color.
    pub error: Color32,
}

impl ViewerLaneTheme {
    /// Derives renderer color roles from egui visuals and a host accent.
    ///
    /// # Parameters
    /// - `visuals`: Current egui visual theme.
    /// - `accent`: Host-selected highlight color.
    pub fn from_visuals(visuals: &egui::Visuals, accent: Color32) -> Self {
        Self {
            background: visuals.extreme_bg_color,
            foreground: visuals.strong_text_color(),
            muted_foreground: visuals.weak_text_color(),
            accent,
            error: visuals.error_fg_color,
        }
    }
}

/// Payload-neutral interaction data supplied by a lane renderer.
///
/// This projection is bounded by the same visible-window request as drawing.
/// It lets generic hover measurement and cursor behavior operate without
/// inspecting an adapter's retained snapshot type.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewerLaneInteraction {
    /// Logic level before the first returned transition.
    pub initial: bool,
    /// Bounded `(timestamp_ns, level_after)` transitions.
    pub transitions: Vec<(u64, bool)>,
    /// Whether the lane represents instantaneous events rather than spans.
    pub event: bool,
}

/// Bounded viewer request accompanying a renderer's interaction projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewerLaneInteractionContext {
    /// First visible timeline timestamp.
    pub visible_start_ns: u64,
    /// Last visible timeline timestamp.
    pub visible_end_ns: u64,
    /// Maximum interaction items the renderer should return.
    pub max_items: usize,
    /// Whether the pointer currently hovers this lane.
    pub hovered: bool,
    /// Timeline timestamp beneath the pointer, when available.
    pub pointer_time_ns: Option<u64>,
}

impl OpaqueLaneDrawContext<'_> {
    /// Maps an absolute timeline position into the clipped waveform region.
    pub fn time_to_x(&self, time_ns: u64) -> f32 {
        let span_ns = self
            .visible_end_ns
            .saturating_sub(self.visible_start_ns)
            .max(1);
        let fraction = time_ns.saturating_sub(self.visible_start_ns) as f64 / span_ns as f64;
        self.wave_rect.left() + self.wave_rect.width() * fraction as f32
    }
}

/// Protocol-neutral extension point for a displayed derived-lane row.
///
/// The viewer retains ownership of waveform queries and drawing geometry.
/// Concrete renderers select row sizing, annotation semantics, and which
/// explicitly registered tracks participate in cursor snapping.
pub trait ViewerLaneRenderer: Send + Sync {
    /// Returns the total row height requested for a lane group.
    ///
    /// # Parameters
    /// - `group`: Tracks and presentation metadata in the lane group.
    /// - `base_height`: Standard viewer-row height before group weighting.
    fn row_height(&self, group: &ViewerLaneGroup, base_height: f32) -> f32 {
        let weight = group
            .tracks
            .iter()
            .map(|track| track.relative_height)
            .sum::<f32>()
            .max(1.0);
        base_height * weight
    }

    /// Selects visual properties for one decoded annotation value.
    ///
    /// # Parameters
    /// - `track`: Track on which the annotation will render.
    /// - `theme`: Viewer color roles.
    /// - `value`: Payload value represented by the annotation.
    /// - `default`: Default visual computed by generic viewer code.
    fn annotation_visual(
        &self,
        _track: &ViewerLaneTrackId,
        _theme: &ViewerLaneTheme,
        _value: u64,
        default: AnnotationVisual,
    ) -> AnnotationVisual {
        default
    }

    /// Draws an adapter-owned opaque lane from an immutable, bounded
    /// snapshot. The viewer invokes this only after releasing all retained
    /// data locks.
    fn draw_opaque_lane(
        &self,
        _track: &ViewerLaneTrack,
        _snapshot: Option<&OpaqueCollectedLaneSnapshot>,
        _context: OpaqueLaneDrawContext<'_>,
    ) -> bool {
        false
    }

    /// Projects a bounded adapter snapshot into generic level/event
    /// transitions for hover measurement. Payloads without level or event
    /// semantics return `None`.
    fn supports_interaction(&self) -> bool {
        false
    }

    /// Projects a bounded opaque snapshot into interaction transitions.
    ///
    /// # Parameters
    /// - `track`: Track whose snapshot is being queried.
    /// - `snapshot`: Immutable bounded retained-data snapshot, if available.
    /// - `context`: Visible window and hover constraints.
    fn interaction(
        &self,
        _track: &ViewerLaneTrack,
        _snapshot: Option<&OpaqueCollectedLaneSnapshot>,
        _context: ViewerLaneInteractionContext,
    ) -> Option<ViewerLaneInteraction> {
        None
    }

    /// Returns tracks whose transitions may be considered for pointer snapping.
    ///
    /// # Parameters
    /// - `group`: Lane group containing the renderer's tracks.
    /// - `pointer_fraction`: Vertical pointer position within the group, from zero to one.
    fn snap_lanes(&self, group: &ViewerLaneGroup, pointer_fraction: f32) -> Vec<DerivedLaneId> {
        let total = group
            .tracks
            .iter()
            .map(|track| track.relative_height)
            .sum::<f32>()
            .max(1.0);
        let target = pointer_fraction.clamp(0.0, 1.0) * total;
        let mut top = 0.0;
        group
            .tracks
            .iter()
            .find(|track| {
                let contains = target >= top && target <= top + track.relative_height;
                top += track.relative_height;
                contains
            })
            .map(|track| vec![track.lane.clone()])
            .unwrap_or_default()
    }
}

/// Default renderer for an ordinary single derived lane.
#[derive(Default)]
pub struct DefaultViewerLaneRenderer;

impl ViewerLaneRenderer for DefaultViewerLaneRenderer {}

/// Compound presentation group that combines one or more derived-data tracks.
pub struct ViewerLaneGroup {
    /// Stable group identity used for persistence and row ordering.
    pub id: ViewerLaneGroupId,
    /// User-facing group label.
    pub label: String,
    /// Badge rendered beside the group label.
    pub badge: ViewerLaneBadge,
    /// Ordered tracks rendered in the group.
    pub tracks: Vec<ViewerLaneTrack>,
    /// Renderer that owns track-specific drawing and interaction projection.
    pub renderer: Arc<dyn ViewerLaneRenderer>,
}

impl fmt::Debug for ViewerLaneGroup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ViewerLaneGroup")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("badge", &self.badge)
            .field("tracks", &self.tracks)
            .finish_non_exhaustive()
    }
}

impl Clone for ViewerLaneGroup {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            label: self.label.clone(),
            badge: self.badge.clone(),
            tracks: self.tracks.clone(),
            renderer: Arc::clone(&self.renderer),
        }
    }
}

impl ViewerLaneGroup {
    /// Creates a one-track group using the default renderer.
    pub fn singleton(
        id: ViewerLaneGroupId,
        label: impl Into<String>,
        badge: ViewerLaneBadge,
        lane: DerivedLaneId,
    ) -> Self {
        Self {
            id,
            label: label.into(),
            badge,
            tracks: vec![ViewerLaneTrack::new("primary", lane, 1.0)],
            renderer: Arc::new(DefaultViewerLaneRenderer),
        }
    }

    /// Divides a group rectangle among tracks by relative height.
    ///
    /// # Parameters
    /// - `top`: Top coordinate of the containing group rectangle.
    /// - `height`: Height of the containing group rectangle.
    pub fn track_rects(&self, top: f32, height: f32) -> Vec<(ViewerLaneTrack, f32, f32)> {
        let total = self
            .tracks
            .iter()
            .map(|track| track.relative_height)
            .sum::<f32>()
            .max(1.0);
        let mut cursor = top;
        self.tracks
            .iter()
            .map(|track| {
                let track_height = height * track.relative_height / total;
                let result = (track.clone(), cursor, track_height);
                cursor += track_height;
                result
            })
            .collect()
    }
}

/// Thread-safe registry of explicit and default derived-lane presentations.
#[derive(Clone)]
pub struct WaveformPresentationRegistry {
    inner: Arc<RwLock<Vec<ViewerLaneGroup>>>,
    defaults: Arc<RwLock<Vec<DefaultPayloadPresentation>>>,
    implicit_groups: Arc<AtomicBool>,
}

struct DefaultPayloadPresentation {
    stable_id: String,
    badge: ViewerLaneBadge,
    renderer: Arc<dyn ViewerLaneRenderer>,
}

impl fmt::Debug for WaveformPresentationRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WaveformPresentationRegistry")
            .field("groups", &self.inner.read().unwrap().len())
            .field("default_payloads", &self.defaults.read().unwrap().len())
            .field("implicit_groups", &self.implicit_groups())
            .finish()
    }
}

impl Default for WaveformPresentationRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::default(),
            defaults: Arc::default(),
            implicit_groups: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl WaveformPresentationRegistry {
    /// Creates an empty presentation registry with implicit groups enabled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers or replaces one explicit compound lane group.
    pub fn register(&self, group: ViewerLaneGroup) {
        let claimed: HashSet<&DerivedLaneId> =
            group.tracks.iter().map(|track| &track.lane).collect();
        let mut groups = self.inner.write().unwrap();
        groups.retain(|existing| {
            existing.id == group.id
                || existing
                    .tracks
                    .iter()
                    .all(|track| !claimed.contains(&track.lane))
        });
        if let Some(existing) = groups.iter_mut().find(|existing| existing.id == group.id) {
            *existing = group;
        } else {
            groups.push(group);
        }
    }

    /// Acquires a read guard over registered compound lane groups.
    pub fn read(&self) -> RwLockReadGuard<'_, Vec<ViewerLaneGroup>> {
        self.inner.read().unwrap()
    }

    /// Removes every explicit compound lane group.
    pub fn clear(&self) {
        self.inner.write().unwrap().clear();
    }

    /// Registers the singleton presentation used when an opaque lane with
    /// this stable payload identity appears without an explicit group.
    ///
    /// # Parameters
    /// - `stable_id`: Payload identity receiving a default presentation.
    /// - `badge`: Badge used for implicit groups of that payload.
    /// - `renderer`: Renderer used for implicit groups of that payload.
    pub fn register_default_payload(
        &self,
        stable_id: impl Into<String>,
        badge: ViewerLaneBadge,
        renderer: Arc<dyn ViewerLaneRenderer>,
    ) {
        let stable_id = stable_id.into();
        let mut defaults = self.defaults.write().unwrap();
        if let Some(existing) = defaults
            .iter_mut()
            .find(|existing| existing.stable_id == stable_id)
        {
            *existing = DefaultPayloadPresentation {
                stable_id,
                badge,
                renderer,
            };
        } else {
            defaults.push(DefaultPayloadPresentation {
                stable_id,
                badge,
                renderer,
            });
        }
    }

    pub(crate) fn default_payload(
        &self,
        stable_id: &str,
    ) -> Option<(ViewerLaneBadge, Arc<dyn ViewerLaneRenderer>)> {
        self.defaults
            .read()
            .unwrap()
            .iter()
            .find(|candidate| candidate.stable_id == stable_id)
            .map(|presentation| {
                (
                    presentation.badge.clone(),
                    Arc::clone(&presentation.renderer),
                )
            })
    }

    /// Controls whether unclaimed retained data appears as a default row.
    /// Graph-driven viewers disable this and subscribe through explicit groups.
    pub fn set_implicit_groups(&self, enabled: bool) {
        self.implicit_groups.store(enabled, Ordering::Relaxed);
    }

    /// Returns whether ungrouped lanes may receive implicit default groups.
    pub fn implicit_groups(&self) -> bool {
        self.implicit_groups.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compound_registration_replaces_overlapping_singletons() {
        let registry = WaveformPresentationRegistry::new();
        let first = DerivedLaneId::new("first");
        let second = DerivedLaneId::new("second");
        let badge = ViewerLaneBadge::new("W", Color32::WHITE);
        registry.register(ViewerLaneGroup::singleton(
            ViewerLaneGroupId::new("first-row"),
            "First",
            badge.clone(),
            first.clone(),
        ));
        registry.register(ViewerLaneGroup::singleton(
            ViewerLaneGroupId::new("second-row"),
            "Second",
            badge.clone(),
            second.clone(),
        ));

        registry.register(ViewerLaneGroup {
            id: ViewerLaneGroupId::new("compound"),
            label: "Compound".to_owned(),
            badge,
            tracks: vec![
                ViewerLaneTrack::new("a", first, 1.0),
                ViewerLaneTrack::new("b", second, 1.0),
            ],
            renderer: Arc::new(DefaultViewerLaneRenderer),
        });

        let groups = registry.read();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id.as_str(), "compound");
        assert_eq!(groups[0].tracks.len(), 2);
    }

    #[test]
    fn default_renderer_snaps_only_the_track_under_the_pointer() {
        let first = DerivedLaneId::new("first");
        let second = DerivedLaneId::new("second");
        let group = ViewerLaneGroup {
            id: ViewerLaneGroupId::new("compound"),
            label: "Compound".to_owned(),
            badge: ViewerLaneBadge::new("W", Color32::WHITE),
            tracks: vec![
                ViewerLaneTrack::new("a", first.clone(), 1.0),
                ViewerLaneTrack::new("b", second.clone(), 1.0),
            ],
            renderer: Arc::new(DefaultViewerLaneRenderer),
        };

        assert_eq!(group.renderer.snap_lanes(&group, 0.2), vec![first]);
        assert_eq!(group.renderer.snap_lanes(&group, 0.8), vec![second]);
    }
}
