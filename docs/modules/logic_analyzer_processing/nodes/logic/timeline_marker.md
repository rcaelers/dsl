# `timeline_marker`

## Responsibility

This module owns processing nodes that introduce, convert, or relate timeline-marker runtime values.

## Boundaries

It does not own host cursor state, graph persistence, or marker rendering. The graph API carries
the neutral marker references and the UI supplies host positions.
