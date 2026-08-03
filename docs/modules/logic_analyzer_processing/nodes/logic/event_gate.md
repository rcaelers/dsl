# `event_gate`

## Responsibility

This module owns level-controlled gating of an event stream.

## Boundaries

It does not own level/event port contracts, viewer presentation, or graph editing; it consumes the
generic runtime contracts supplied by `signal_processing`.
