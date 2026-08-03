# `logic_analyzer_processing::nodes::logic`

## Responsibility

This namespace groups concrete protocol-independent stream transformation and control nodes.

## Child owners

- [buffer](logic/buffer.md), [edge detector](logic/edge_detector.md), and
  [event control](logic/event_control.md)
- [event gate](logic/event_gate.md), [logic gate](logic/logic_gate.md), and
  [packet framer](logic/packet_framer.md)
- [SR latch](logic/sr_latch.md), [text formatter](logic/text_formatter.md), and
  [timeline marker](logic/timeline_marker.md)
- [trigger counter](logic/trigger_counter.md), [word field extractor](logic/word_field_extractor.md),
  and [word matcher](logic/word_matcher.md)

## Boundaries

Each child owns one runtime transformation. Generic scheduling and ports remain in
`signal_processing`; graph-node definitions and UI presentation remain above processing.
