# `node_graph::api`

## Responsibility

This namespace owns the graph-document, node-definition, socket-definition, control, and portable
file-dialog contracts consumed by graph-node features and graph compilation.

## Boundaries

It does not expose editor implementation operations, concrete payload types, compiler policy, or a
native/web dialog backend. The crate root remains the editor-widget composition surface.
