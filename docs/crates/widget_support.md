# `widget_support` Design

## Responsibility

`widget_support` owns small generic egui support primitives shared by reusable widgets.

## Boundaries

It has no workspace dependency and contains no application, graph, capture, protocol, or host
policy. A helper moves into a more specific widget or a neutral lower-level crate when this broad
support responsibility no longer describes it.
