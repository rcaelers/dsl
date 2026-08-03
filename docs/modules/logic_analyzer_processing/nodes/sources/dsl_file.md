# `dsl_file`

## Responsibility

This module owns DSL archive parsing, prepared-source indexing, and replay processing behavior.

## Boundaries

Host path acquisition is an explicitly allowlisted compatibility boundary; normal composition
injects prepared byte sources. Graph definitions, file dialogs, and viewer attachment remain above
this processing source.
