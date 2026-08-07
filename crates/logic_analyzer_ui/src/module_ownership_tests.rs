fn module_documentation(source: &'static str) -> String {
    source
        .lines()
        .take_while(|line| line.starts_with("//!"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn substantial_ui_modules_answer_the_four_ownership_questions() {
    for (owner, source) in [
        ("application platform", include_str!("app_platform/mod.rs")),
        ("decoder panel", include_str!("decoder_panel/mod.rs")),
        ("headless execution", include_str!("headless.rs")),
        ("viewer selection", include_str!("viewer_selection.rs")),
    ] {
        let documentation = module_documentation(source);
        for heading in [
            "**Owned data and invariants.**",
            "**Facade.**",
            "**Permitted owner dependencies.**",
            "**Explicit exclusions.**",
        ] {
            assert!(
                documentation.contains(heading),
                "{owner} module documentation is missing {heading}"
            );
        }
    }
}

#[test]
fn substantial_ui_owner_fields_are_private() {
    for (owner, type_name, source) in [
        (
            "application platform",
            "PlatformState",
            include_str!("app_platform/state.rs"),
        ),
        (
            "decoder panel",
            "DecoderPanels",
            include_str!("decoder_panel/implementation.rs"),
        ),
        (
            "headless execution",
            "HeadlessGraphRunner",
            include_str!("headless.rs"),
        ),
    ] {
        let declaration = source
            .split_once(&format!("struct {type_name} {{"))
            .unwrap_or_else(|| panic!("{owner} declaration"))
            .1
            .split_once("\n}")
            .unwrap_or_else(|| panic!("{owner} declaration end"))
            .0;
        assert!(
            !declaration.contains("pub(crate) ") && !declaration.contains("pub "),
            "{owner} exposes mutable state instead of methods"
        );
    }
}
