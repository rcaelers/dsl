# `signal_processing::capture`

## Responsibility

This module owns generic immutable capture-source, index, query, and worker-operation contracts.
It describes sampled data and prepared random access without naming a file format, device, path, or
viewer.

## Boundaries

Concrete parsers and source nodes implement these contracts above `signal_processing`. Artifact
backing is supplied through generic storage contracts; waveform summary construction belongs to
`waveform_index`.
