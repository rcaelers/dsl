use node_graph::api::EnumValue;

const WORD_DISPLAY_FORMATS: &[&str] =
    &["Hex", "Binary", "Octal", "Decimal", "ASCII", "Hex + ASCII"];

pub(crate) fn default_word_display_format() -> EnumValue {
    EnumValue::new(0, WORD_DISPLAY_FORMATS)
}
