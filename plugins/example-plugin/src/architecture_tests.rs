#[test]
fn graph_nodes_register_narrow_capabilities() {
    let pulse_measure = include_str!("pulse_measure.rs");
    let camera_frame = include_str!("camera_frame.rs");

    for source in [pulse_measure, camera_frame] {
        assert!(source.contains("impl GraphNodeSemantics"));
        assert!(source.contains("impl RuntimeMaterializer"));
        assert!(source.contains("GraphNodeRegistration::capable"));
        assert!(!source.contains("RuntimeBuilder"));
        assert!(!source.contains("GraphNodeRegistration::runnable"));
    }
}
