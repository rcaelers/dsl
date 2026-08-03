# `logic_analyzer_ui` Design

## Responsibility

`logic_analyzer_ui` composes portable application panels, graph editing, viewer presentation,
capture coordination, preferences, and user-facing services. It owns UI state and document-facing
migrations such as viewer-output selections.

## Facade and dependencies

The root exposes the application, app-service bundle, host and export ports, node registry,
headless runner, and panel-plugin contracts. Its private graph-service port isolates the concrete
compiler/run implementation from UI tests. It depends on generic widgets, graph API/compiler, and
generic signal contracts.

## Ownership boundaries

The UI does not define concrete processing or graph-node features, select native/web behavior,
directly access host paths, or own pipeline scheduling policy. `logic_analyzer_platform` implements
its host ports, while app crates only bootstrap that composition.
