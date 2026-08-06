#[test]
fn device_protocol_uses_an_injected_transport_contract() {
    let module = include_str!("device/dslogic_u3pro16/mod.rs");
    let implementation = include_str!("device/dslogic_u3pro16/implementation.rs");
    let transport = include_str!("device/dslogic_u3pro16/transport.rs");

    assert!(module.contains("mod transport;"));
    assert!(!module.contains("mod platform;"));
    assert!(transport.contains("pub trait DsLogicU3Pro16TransportFactory"));
    assert!(transport.contains("pub trait UsbTransport"));
    assert!(transport.contains("fn fpga_image("));
    assert!(!implementation.contains("RusbTransport"));
    assert!(!implementation.contains("rusb::"));
    assert!(!implementation.contains("std::fs"));
    assert!(!implementation.contains("std::env"));
}
