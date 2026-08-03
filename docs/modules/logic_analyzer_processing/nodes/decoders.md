# `logic_analyzer_processing::nodes::decoders`

## Responsibility

This namespace groups concrete protocol-decoder processing nodes.

## Child owners

- [I²C decoder](decoders/i2c_decoder.md)
- [parallel decoder](decoders/parallel_decoder.md)
- [Sigrok decoder](decoders/sigrok_decoder.md)
- [SPI decoder](decoders/spi_decoder.md)
- [UART decoder](decoders/uart_decoder.md)

## Boundaries

Each child owns one protocol state machine and its UI-independent configuration. Graph definitions,
socket labels, renderer metadata, and host runtime selection remain outside this namespace.
