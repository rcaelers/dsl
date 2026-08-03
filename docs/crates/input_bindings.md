# `input_bindings` Design

## Responsibility

`input_bindings` owns portable shortcut, modifier, and input-binding representation shared by
widgets and application UI.

## Boundaries

It depends only on `egui` and serialization. It does not own menu layout, panel policy, concrete
commands, host keyboard labels, or target selection. Consumers map its binding values to their own
actions and persist them through their owning settings model.
