# `synthetic_uart_source`

## Responsibility

This module owns an explicit portable synthetic UART signal source for authored demos and tests.

## Boundaries

It does not emulate an unavailable hardware source implicitly and does not own UART graph
presentation or decoder configuration.
