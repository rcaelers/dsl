# `logic_analyzer_processing::types`

## Responsibility

This namespace owns protocol-neutral processing value conventions shared by concrete processing
nodes.

## Boundaries

It does not own generic runtime payload contracts, graph sockets, widget presentation, or concrete
source/decoder behavior. Values with wider generic meaning remain in `signal_processing`.
