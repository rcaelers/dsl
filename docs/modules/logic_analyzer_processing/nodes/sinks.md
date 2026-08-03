# `logic_analyzer_processing::nodes::sinks`

## Responsibility

This namespace groups concrete terminal processing nodes that persist, discard, or record streams.

## Child owners

- [binary writer](sinks/binary_file_writer.md), [CSV word writer](sinks/csv_word_writer.md), and
  [discard writer](sinks/discard_writer.md)
- [output storage contract](sinks/output_storage.md), [text writer](sinks/text_file_writer.md), and
  [TGCK recorder](sinks/tgck_recorder.md)

## Boundaries

Each sink owns output semantics and format behavior. Destination acquisition and target-specific
file access are supplied through host-injected storage contracts.
