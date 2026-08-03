# `sigrok_file`

## Responsibility

This module owns Sigrok archive parsing, prepared-source indexing, and replay processing behavior.

## Boundaries

Host path acquisition is an explicitly allowlisted compatibility boundary; normal composition
supplies prepared sources. Sigrok graph-node configuration and UI presentation remain elsewhere.
