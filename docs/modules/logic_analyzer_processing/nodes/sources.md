# `logic_analyzer_processing::nodes::sources`

## Responsibility

This namespace groups concrete file, device, and synthetic processing sources.

## Child owners

- [DSL file](sources/dsl_file.md), [DSLogic U3Pro16](sources/dslogic_u3pro16.md), and
  [Sigrok file](sources/sigrok_file.md)
- [synthetic capture](sources/synthetic_capture_source.md) and
  [synthetic UART](sources/synthetic_uart_source.md)

## Boundaries

Sources own portable parsing and capture behavior. Host paths, USB transport, browser handles, and
source selection are injected through explicit contracts rather than selected by target code here.
