use logic_analyzer_processing::nodes::decoders::sigrok_decoder::{
    SigrokAnnotationClassDescriptor, SigrokAnnotationRowDescriptor, SigrokDecoderChannelDescriptor,
    SigrokDecoderDescriptor, SigrokOutputKind,
};

pub(crate) fn test_sigrok_logic_descriptor() -> SigrokDecoderDescriptor {
    SigrokDecoderDescriptor {
        api_version: 3,
        id: "test_logic".into(),
        name: "Test Logic".into(),
        long_name: "Test Logic Decoder".into(),
        description: "Deterministic raw-logic decoder contract".into(),
        license: "mit".into(),
        inputs: vec!["logic".into()],
        outputs: vec!["test_logic".into()],
        tags: vec!["Test".into()],
        channels: vec![channel("mosi", "MOSI")],
        optional_channels: vec![channel("cs", "CS")],
        options: Vec::new(),
        annotations: vec![annotation("data", "Data")],
        annotation_rows: vec![SigrokAnnotationRowDescriptor {
            id: "data".into(),
            description: "Data".into(),
            classes: vec![0],
        }],
        binary: vec![annotation("binary", "Binary")],
        logic_output_channels: vec![channel("generated", "Generated")],
        registered_outputs: vec![
            SigrokOutputKind::Annotation,
            SigrokOutputKind::Binary,
            SigrokOutputKind::GeneratedLogic,
            SigrokOutputKind::Metadata,
            SigrokOutputKind::ProtocolPacket,
        ],
        package_fingerprint: "test-logic-fingerprint".into(),
    }
}

pub(crate) fn test_sigrok_stacked_descriptor() -> SigrokDecoderDescriptor {
    SigrokDecoderDescriptor {
        api_version: 3,
        id: "test_stacked".into(),
        name: "Test Stacked".into(),
        long_name: "Test Stacked Decoder".into(),
        description: "Deterministic stacked-decoder contract".into(),
        license: "mit".into(),
        inputs: vec!["test_logic".into()],
        outputs: vec!["test_stacked".into()],
        tags: vec!["Test".into()],
        channels: Vec::new(),
        optional_channels: Vec::new(),
        options: Vec::new(),
        annotations: vec![annotation("result", "Result")],
        annotation_rows: vec![SigrokAnnotationRowDescriptor {
            id: "result".into(),
            description: "Result".into(),
            classes: vec![0],
        }],
        binary: Vec::new(),
        logic_output_channels: Vec::new(),
        registered_outputs: vec![
            SigrokOutputKind::Annotation,
            SigrokOutputKind::ProtocolPacket,
        ],
        package_fingerprint: "test-stacked-fingerprint".into(),
    }
}

fn channel(id: &str, name: &str) -> SigrokDecoderChannelDescriptor {
    SigrokDecoderChannelDescriptor {
        id: id.into(),
        name: name.into(),
        description: name.into(),
    }
}

fn annotation(id: &str, description: &str) -> SigrokAnnotationClassDescriptor {
    SigrokAnnotationClassDescriptor {
        id: id.into(),
        description: description.into(),
    }
}
