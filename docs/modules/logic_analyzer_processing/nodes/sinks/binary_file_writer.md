# `binary_file_writer`

## Responsibility

This module owns streaming binary persistence of its configured runtime payloads.

## Boundaries

It receives writable output through `OutputStorage`; it does not acquire a path, open a native
dialog, select a browser download, or define graph-node controls.
