# `output_storage`

## Responsibility

This internal module owns the processing-facing contract for opening and writing output files.

## Boundaries

It names no native path, browser handle, dialog, or output-selection policy. Platform composition
implements the contract and concrete sink nodes consume it.
