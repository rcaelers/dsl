//! Metadata-driven graph feature for one Sigrok Python decoder instance.

mod builder;
mod definition;
mod registration;

pub(crate) use builder::runtime_builder_override;
pub(crate) use definition::node_templates;
