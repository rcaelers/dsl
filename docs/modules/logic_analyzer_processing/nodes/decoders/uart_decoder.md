# `uart_decoder`

## Responsibility

This module owns UART signal decoding and UART word/diagnostic production from generic sampled
inputs.

## Boundaries

It does not define UART node sockets, panel controls, display presentation, or host scheduling.
Those concerns belong to the graph-node, UI, and runtime owners.
