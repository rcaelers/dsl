#[test]
fn file_sinks_use_an_injected_destination_capability() {
    for facade in [
        include_str!("binary_file_writer/facade.rs"),
        include_str!("csv_word_writer/facade.rs"),
        include_str!("text_file_writer/facade.rs"),
    ] {
        assert!(facade.contains("Arc<dyn OutputStorage>"));
        assert!(facade.contains("unavailable_writer_factory"));
        assert!(!facade.contains("super::platform"));
    }

    for implementation in [
        include_str!("binary_file_writer/implementation.rs"),
        include_str!("csv_word_writer/implementation.rs"),
        include_str!("text_file_writer/implementation.rs"),
        include_str!("output_storage/implementation.rs"),
    ] {
        let production = implementation
            .split_once("#[cfg(test)]")
            .map_or(implementation, |(production, _)| production);
        assert!(!production.contains("std::fs"));
        assert!(!production.contains("File::create"));
        assert!(!production.contains("OpenOptions"));
    }
}
