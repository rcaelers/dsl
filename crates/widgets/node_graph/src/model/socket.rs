use std::collections::BTreeMap;

use egui::Color32;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::ids::SocketDirection;

/// Visual shape used to distinguish socket families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SocketShape {
    #[default]
    /// Round socket, the default stream-data shape.
    Circle,
    /// Diamond socket.
    Diamond,
    /// Square socket, used by built-in configuration values.
    Square,
    /// Triangle socket.
    Triangle,
}

/// State of a socket that belongs to a growing input group. A `placeholder`
/// waits for a connection; connecting converts it to a member and spawns a
/// fresh placeholder (until `max` members exist). Disconnecting a member
/// removes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariadicInfo {
    /// Group label from the def; members display as "{base} {n}".
    pub base: String,
    /// Maximum number of members.
    pub max: usize,
    /// Whether this is the unconnected member slot that can grow the group.
    pub placeholder: bool,
}

/// A socket on a node instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Socket {
    /// Stable owner-defined identity within one node instance schema.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub schema_id: String,
    #[serde(default, skip_serializing)]
    /// User-facing label, derived from the owning definition.
    pub name: String,
    /// Native type. For inputs this is what the node primarily expects; the
    /// socket may temporarily resolve to one of `allowed` while connected.
    #[serde(default = "default_socket_type_name", skip_serializing)]
    pub type_name: String,
    /// Idle look, owned by the node definition (`idle_style` / `on_update`).
    /// The resolved look is derived from the type identity table at render time.
    #[serde(default = "default_socket_color", skip_serializing)]
    pub color: Color32,
    #[serde(default, skip_serializing)]
    /// Definition-provided visual shape for the socket family.
    pub shape: SocketShape,
    /// Additional type names this input accepts besides `type_name`. The node
    /// declared it can handle these itself. Empty = strict.
    #[serde(default, skip_serializing)]
    pub allowed: Vec<String>,
    /// Set while connected to an output whose type differs from `type_name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_type: Option<String>,
    /// Which `InputDef`/`OutputDef` of the node definition this socket came
    /// from. Socket and def counts diverge once variadic groups grow, so all
    /// def lookups (controls, restore) go through this instead of position.
    /// Removed stable-ID sockets use `usize::MAX` internally; its serialized
    /// `u64::MAX` sentinel maps back to the local pointer width when loaded.
    #[serde(
        default,
        skip_serializing_if = "is_zero",
        serialize_with = "serialize_def_index",
        deserialize_with = "deserialize_def_index"
    )]
    pub def_index: usize,
    /// Present when this socket belongs to a variadic group.
    #[serde(default, skip_serializing)]
    pub variadic: Option<VariadicInfo>,
    /// Controlled by `on_update` — set false to suppress the socket entirely.
    #[serde(default = "default_true", skip_serializing)]
    pub visible: bool,
    /// Definition-owned node-editor presentation. Unlike `visible`, this does
    /// not remove the output from host processing.
    #[serde(default = "default_true", skip_serializing)]
    pub editor_visible: bool,
    /// Set true by the user via "Hide Unused"; never touched by `on_update`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    #[serde(default, skip_serializing)]
    /// Whether an unconnected input can render an inline control.
    pub has_control: bool,
    /// Opaque owner-managed metadata local to this socket. Values travel with
    /// node fragments during copy/paste and survive definition reconciliation
    /// by `schema_id`; graph-wide references belong in `GraphState` extensions.
    /// Generic graph code only preserves these values.
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

fn default_socket_type_name() -> String {
    "Any".to_owned()
}

fn default_socket_color() -> Color32 {
    Color32::from_rgb(150, 150, 150)
}

fn default_true() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !value
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

fn serialize_def_index<S>(value: &usize, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let portable = if *value == usize::MAX {
        u64::MAX
    } else {
        *value as u64
    };
    serializer.serialize_u64(portable)
}

fn deserialize_def_index<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: Deserializer<'de>,
{
    let portable = u64::deserialize(deserializer)?;
    if portable == u64::MAX {
        return Ok(usize::MAX);
    }
    usize::try_from(portable).map_err(|_| {
        D::Error::custom(format!(
            "socket definition index {portable} exceeds this platform's index range"
        ))
    })
}

impl Socket {
    /// The type this socket currently carries: the connected type while
    /// resolved, the native type otherwise.
    pub fn effective_type(&self) -> &str {
        self.resolved_type.as_deref().unwrap_or(&self.type_name)
    }

    /// Whether this input socket accepts a connection from an output of
    /// `incoming` type. Acceptance is per-socket (declared by the node),
    /// not a property of the socket type.
    ///
    /// # Parameters
    /// - `incoming`: Input consumed by this operation.
    pub fn accepts(&self, incoming: &str) -> bool {
        incoming == "Any"
            || self.type_name == "Any"
            || incoming == self.type_name
            || self.allowed.iter().any(|t| t == incoming)
    }

    /// Returns whether variadic placeholder.
    pub fn is_variadic_placeholder(&self) -> bool {
        self.variadic.as_ref().is_some_and(|info| info.placeholder)
    }

    /// Returns whether variadic member.
    pub fn is_variadic_member(&self) -> bool {
        self.variadic.as_ref().is_some_and(|info| !info.placeholder)
    }

    /// Whether a socket in direction `from_dir` could connect to `to` in
    /// direction `to_dir` — the same output-accepts-input check `accepts`
    /// performs, usable before either socket has a [`super::ids::SocketId`] (e.g.
    /// a freshly instantiated, not-yet-added node being probed for
    /// compatibility with a dragged wire).
    pub fn compatible(
        from: &Socket,
        from_dir: SocketDirection,
        to: &Socket,
        to_dir: SocketDirection,
    ) -> bool {
        if from_dir == to_dir {
            return false;
        }
        let (output, input) = if from_dir == SocketDirection::Output {
            (from, to)
        } else {
            (to, from)
        };
        input.accepts(output.effective_type())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn socket(type_name: &str) -> Socket {
        Socket {
            schema_id: String::new(),
            name: String::new(),
            type_name: type_name.to_owned(),
            color: Color32::WHITE,
            shape: SocketShape::Circle,
            allowed: Vec::new(),
            resolved_type: None,
            def_index: 0,
            variadic: None,
            visible: true,
            editor_visible: true,
            hidden: false,
            has_control: false,
            extensions: Default::default(),
        }
    }

    #[test]
    fn compatible_pairs_output_with_accepting_input() {
        let output = socket("Float");
        let input = socket("Float");
        assert!(Socket::compatible(
            &output,
            SocketDirection::Output,
            &input,
            SocketDirection::Input
        ));
        assert!(Socket::compatible(
            &input,
            SocketDirection::Input,
            &output,
            SocketDirection::Output
        ));
    }

    #[test]
    fn compatible_rejects_same_direction_and_mismatched_types() {
        let a = socket("Float");
        let b = socket("Int");
        assert!(!Socket::compatible(
            &a,
            SocketDirection::Output,
            &b,
            SocketDirection::Output
        ));
        assert!(!Socket::compatible(
            &a,
            SocketDirection::Output,
            &b,
            SocketDirection::Input
        ));
    }

    #[test]
    fn compatible_any_type_matches_everything() {
        let any_output = socket("Any");
        let typed_input = socket("Float");
        assert!(Socket::compatible(
            &any_output,
            SocketDirection::Output,
            &typed_input,
            SocketDirection::Input
        ));
    }

    #[test]
    fn definition_owned_fields_are_not_serialized_and_host_values_are_opaque() {
        let socket = socket("Float");
        let mut value = serde_json::to_value(&socket).unwrap();
        let object = value.as_object_mut().unwrap();
        assert!(!object.contains_key("name"));
        assert!(!object.contains_key("type_name"));
        assert!(!object.contains_key("color"));
        assert!(!object.contains_key("shape"));
        assert!(!object.contains_key("allowed"));
        assert!(!object.contains_key("variadic"));
        assert!(!object.contains_key("resolved_type"));
        assert!(!object.contains_key("visible"));
        assert!(!object.contains_key("hidden"));
        assert!(!object.contains_key("has_control"));

        object.insert("name".to_owned(), serde_json::json!("Input"));
        object.insert("type_name".to_owned(), serde_json::json!("Float"));
        object.insert("color".to_owned(), serde_json::json!([1, 2, 3, 255]));
        object.insert("shape".to_owned(), serde_json::json!("Diamond"));
        object.insert("host.selection".to_owned(), serde_json::json!(false));
        object.insert(
            "variadic".to_owned(),
            serde_json::json!({"base":"Input","max":4,"placeholder":true}),
        );
        let restored: Socket = serde_json::from_value(value).unwrap();
        assert_eq!(restored.name, "Input");
        assert_eq!(restored.type_name, "Float");
        assert_eq!(restored.color, Color32::from_rgb(1, 2, 3));
        assert_eq!(restored.shape, SocketShape::Diamond);
        assert_eq!(
            restored.extensions.get("host.selection"),
            Some(&serde_json::json!(false))
        );
        assert!(restored.is_variadic_placeholder());
    }

    #[test]
    fn removed_socket_definition_index_has_a_platform_independent_sentinel() {
        let mut removed = socket("Trigger");
        removed.schema_id = "removed-output".to_owned();
        removed.def_index = usize::MAX;

        let value = serde_json::to_value(&removed).unwrap();
        assert_eq!(value["def_index"], serde_json::json!(u64::MAX));

        let restored: Socket = serde_json::from_value(value).unwrap();
        assert_eq!(restored.def_index, usize::MAX);
    }
}
