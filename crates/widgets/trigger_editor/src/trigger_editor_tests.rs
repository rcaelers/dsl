use logic_analyzer_trigger::{
    RegisteredTriggerPredicateSchema, SimpleTriggerCondition, TriggerChoice, TriggerCount,
    TriggerCountCapabilities, TriggerCountMode, TriggerEditorSchema, TriggerIdentifier,
    TriggerLogicOperator, TriggerOperandKind, TriggerOperandSchema, TriggerOperandValue,
    TriggerPredicate,
};
use signal_capture::CaptureChannelId;

use super::presentation::{format_bytes, parse_bytes};
use super::*;

fn id(value: &str) -> TriggerIdentifier {
    TriggerIdentifier::new(value).unwrap()
}

fn schema() -> TriggerEditorSchema {
    TriggerEditorSchema::new(
        id("test.editor"),
        1,
        2,
        4,
        vec![TriggerLogicOperator::And, TriggerLogicOperator::Or],
    )
    .unwrap()
    .with_digital_conditions(vec![
        SimpleTriggerCondition::High,
        SimpleTriggerCondition::Rising,
    ])
    .unwrap()
    .with_stage_inversion(true)
    .with_count(
        TriggerCountCapabilities::new(vec![TriggerCountMode::Occurrences], 1, 9, 1).unwrap(),
    )
    .with_registered_predicates(vec![
        RegisteredTriggerPredicateSchema::new(
            id("test.sequence"),
            "Sequence",
            vec![
                TriggerOperandSchema::new(
                    id("channel"),
                    "Channel",
                    TriggerOperandKind::Channel { default: None },
                )
                .unwrap(),
                TriggerOperandSchema::new(
                    id("value"),
                    "Value",
                    TriggerOperandKind::Unsigned {
                        minimum: 0,
                        maximum: 255,
                        step: 1,
                        default: 0,
                    },
                )
                .unwrap(),
                TriggerOperandSchema::new(
                    id("enabled"),
                    "Enabled",
                    TriggerOperandKind::Boolean { default: true },
                )
                .unwrap(),
                TriggerOperandSchema::new(
                    id("offset"),
                    "Offset",
                    TriggerOperandKind::Signed {
                        minimum: -8,
                        maximum: 8,
                        step: 2,
                        default: 0,
                    },
                )
                .unwrap(),
                TriggerOperandSchema::new(
                    id("duration"),
                    "Duration",
                    TriggerOperandKind::DurationNs {
                        minimum: 100,
                        maximum: 1_000,
                        step: 100,
                        default: 100,
                    },
                )
                .unwrap(),
                TriggerOperandSchema::new(
                    id("edge"),
                    "Edge",
                    TriggerOperandKind::Choice {
                        choices: vec![
                            TriggerChoice::new(id("rise"), "Rise").unwrap(),
                            TriggerChoice::new(id("fall"), "Fall").unwrap(),
                        ],
                        default: id("rise"),
                    },
                )
                .unwrap(),
                TriggerOperandSchema::new(
                    id("bytes"),
                    "Bytes",
                    TriggerOperandKind::Bytes {
                        minimum_length: 1,
                        maximum_length: 4,
                        default: vec![0],
                    },
                )
                .unwrap(),
            ],
        )
        .unwrap(),
    ])
    .unwrap()
}

fn channels() -> Vec<TriggerEditorChannel> {
    vec![
        TriggerEditorChannel {
            id: CaptureChannelId::new("bank-a:7"),
            label: "A7".into(),
        },
        TriggerEditorChannel {
            id: CaptureChannelId::new("bank-z:41"),
            label: "Z41".into(),
        },
    ]
}

#[test]
fn neutral_actions_build_stages_counts_and_registered_operands() {
    let schema = schema();
    let channels = channels();
    let model = TriggerEditorModel::new(&schema, &channels);
    let mut program = model.apply(None, TriggerEditorAction::AddStage).unwrap();
    program = model
        .apply(
            program.as_ref(),
            TriggerEditorAction::AddRegisteredPredicate {
                stage: 0,
                predicate: id("test.sequence"),
            },
        )
        .unwrap();
    for (operand, value) in [
        (id("enabled"), TriggerOperandValue::Boolean(false)),
        (id("offset"), TriggerOperandValue::Signed(-2)),
        (id("duration"), TriggerOperandValue::DurationNs(300)),
        (id("edge"), TriggerOperandValue::Choice(id("fall"))),
        (
            id("channel"),
            TriggerOperandValue::Channel(channels[1].id.clone()),
        ),
        (id("bytes"), TriggerOperandValue::Bytes(vec![0x5a, 0xa5])),
    ] {
        program = model
            .apply(
                program.as_ref(),
                TriggerEditorAction::SetRegisteredOperand {
                    stage: 0,
                    predicate: 1,
                    operand,
                    value,
                },
            )
            .unwrap();
    }
    program = model
        .apply(
            program.as_ref(),
            TriggerEditorAction::SetStageCount {
                stage: 0,
                count: Some(TriggerCount {
                    mode: TriggerCountMode::Occurrences,
                    value: 4,
                }),
            },
        )
        .unwrap();
    assert!(
        model
            .apply(
                program.as_ref(),
                TriggerEditorAction::AddDigitalPredicate {
                    stage: 0,
                    channel: channels[0].id.clone(),
                    condition: SimpleTriggerCondition::Rising,
                },
            )
            .unwrap_err()
            .contains("more than once")
    );
    program = model
        .apply(
            program.as_ref(),
            TriggerEditorAction::SetRegisteredOperand {
                stage: 0,
                predicate: 1,
                operand: id("value"),
                value: TriggerOperandValue::Unsigned(0x5a),
            },
        )
        .unwrap();
    program = model
        .apply(program.as_ref(), TriggerEditorAction::AddStage)
        .unwrap();

    let program = program.unwrap();
    assert_eq!(program.stages.len(), 2);
    assert_eq!(program.stages[0].count.unwrap().value, 4);
    let TriggerPredicate::Registered { operands, .. } = &program.stages[0].predicates[1] else {
        panic!("second predicate should be registered");
    };
    assert_eq!(operands[&id("value")], TriggerOperandValue::Unsigned(0x5a));
    assert_eq!(
        operands[&id("bytes")],
        TriggerOperandValue::Bytes(vec![0x5a, 0xa5])
    );
    schema
        .validate_program(
            &program,
            &channels
                .iter()
                .map(|channel| channel.id.clone())
                .collect::<Vec<_>>(),
        )
        .unwrap();

    let without_registered = model
        .apply(
            Some(&program),
            TriggerEditorAction::RemovePredicate {
                stage: 0,
                predicate: 1,
            },
        )
        .unwrap();
    let without_count = model
        .apply(
            without_registered.as_ref(),
            TriggerEditorAction::SetStageCount {
                stage: 0,
                count: None,
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(without_count.stages[0].predicates.len(), 1);
    assert!(without_count.stages[0].count.is_none());
}

#[test]
fn neutral_actions_enforce_schema_limits_and_clear_invalid_programs() {
    let schema = schema();
    let channels = channels();
    let model = TriggerEditorModel::new(&schema, &channels);
    let first = model.apply(None, TriggerEditorAction::AddStage).unwrap();
    let second = model
        .apply(first.as_ref(), TriggerEditorAction::AddStage)
        .unwrap();
    assert!(
        model
            .apply(second.as_ref(), TriggerEditorAction::AddStage)
            .unwrap_err()
            .contains("at most 2")
    );

    let mut incompatible = second.unwrap();
    incompatible.schema_revision = 99;
    assert!(
        model
            .apply(Some(&incompatible), TriggerEditorAction::AddStage)
            .is_err()
    );
    assert_eq!(
        model
            .apply(Some(&incompatible), TriggerEditorAction::Clear)
            .unwrap(),
        None
    );
}

#[test]
fn neutral_actions_edit_and_remove_digital_predicates_and_stages() {
    let schema = schema();
    let channels = channels();
    let model = TriggerEditorModel::new(&schema, &channels);
    let mut program = model.apply(None, TriggerEditorAction::AddStage).unwrap();
    program = model
        .apply(
            program.as_ref(),
            TriggerEditorAction::SetDigitalChannel {
                stage: 0,
                predicate: 0,
                channel: channels[1].id.clone(),
            },
        )
        .unwrap();
    program = model
        .apply(
            program.as_ref(),
            TriggerEditorAction::SetDigitalCondition {
                stage: 0,
                predicate: 0,
                condition: SimpleTriggerCondition::Rising,
            },
        )
        .unwrap();
    program = model
        .apply(
            program.as_ref(),
            TriggerEditorAction::AddDigitalPredicate {
                stage: 0,
                channel: channels[0].id.clone(),
                condition: SimpleTriggerCondition::High,
            },
        )
        .unwrap();
    program = model
        .apply(
            program.as_ref(),
            TriggerEditorAction::SetStageLogic {
                stage: 0,
                logic: TriggerLogicOperator::Or,
            },
        )
        .unwrap();
    program = model
        .apply(
            program.as_ref(),
            TriggerEditorAction::SetStageInverted {
                stage: 0,
                inverted: true,
            },
        )
        .unwrap();
    program = model
        .apply(
            program.as_ref(),
            TriggerEditorAction::RemovePredicate {
                stage: 0,
                predicate: 1,
            },
        )
        .unwrap();
    let program_ref = program.as_ref().unwrap();
    assert_eq!(program_ref.stages[0].logic, TriggerLogicOperator::Or);
    assert!(program_ref.stages[0].inverted);
    assert_eq!(program_ref.stages[0].predicates.len(), 1);

    assert_eq!(
        model
            .apply(
                program.as_ref(),
                TriggerEditorAction::RemoveStage { stage: 0 },
            )
            .unwrap(),
        None
    );
}

#[test]
fn byte_values_use_bounded_hex_pairs() {
    assert_eq!(parse_bytes("00 5a FF"), Some(vec![0, 0x5a, 0xff]));
    assert_eq!(parse_bytes("5"), Some(vec![5]));
    assert_eq!(parse_bytes("xyz"), None);
    assert_eq!(format_bytes(&[0, 0x5a, 0xff]), "00 5A FF");
}
