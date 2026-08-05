#[test]
fn native_catalog_owns_paths_selection_persistence_and_scanning() {
    let catalog = include_str!("platform/native_sigrok/catalog.rs");

    assert!(catalog.contains("PathBuf"));
    assert!(catalog.contains("rfd::FileDialog"));
    assert!(catalog.contains("SavedSettings"));
    assert!(catalog.contains("scan_catalog"));
    assert!(catalog.contains("impl NodeCatalogService"));
}
