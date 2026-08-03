# `tgck_recorder`

## Responsibility

This module owns TGCK recorder output for its configured processing stream.

## Boundaries

It owns recorder encoding, not graph-node state, file destination acquisition, or target-specific
storage implementation.
