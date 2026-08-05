#[test]
fn catalog_contract_is_portable() {
    let contract = include_str!("contract.rs");
    assert!(contract.contains("pub trait NodeCatalogService"));
    assert!(contract.contains("pub struct NodeCatalogSnapshot"));
    assert!(!contract.contains("PathBuf"));
    assert!(!contract.contains("rfd::"));
}
