# `panel_layout` Design

## Responsibility

`panel_layout` owns reusable panel placement, docking, resizing, visibility, and persisted layout
behavior for egui applications.

## Boundaries

It depends on generic widget and input support plus serialization. It does not define application
panels, menu commands, concrete graph nodes, capture state, or host persistence policy. The UI
injects panel identities and content through its composition boundary.
