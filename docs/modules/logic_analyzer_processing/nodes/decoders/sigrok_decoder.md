# `sigrok_decoder`

## Responsibility

This module owns portable Sigrok decoder configuration, execution behavior, and output contracts.

## Boundaries

Python-host discovery, interpreter setup, package locations, and concrete execution factories are
platform concerns injected through its contracts. The module does not own graph-node presentation.
