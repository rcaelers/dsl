# `logic_analyzer_graph_api::node_support`

## Responsibility

This namespace owns value contracts supplied to graph-node implementations: port identities,
resolved inputs, restricted build context, state decoding, and presentation/capture descriptors.

## Boundaries

It contains no editor widget, compiler lifecycle, concrete node behavior, target selection, or host
path handling. Descriptors carry stable metadata rather than requiring generic consumers to infer
behavior from display names.
