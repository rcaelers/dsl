# `signal_processing::live_capture_store`

## Responsibility

This module owns the authoritative append-only storage and replay contracts for a live capture
session, including committed-prefix visibility and finalized-session access.

## Boundaries

It uses generic artifact storage and contains no device protocol, path policy, UI state, or derived
processing policy. Capture coordination decides session replacement and presentation attachment.
