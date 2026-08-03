# `signal_processing::live_capture`

## Responsibility

This module owns driver-neutral acquisition configuration, commands, events, progress, queueing,
and terminal outcomes for a live capture session.

## Boundaries

It does not implement a device transport, create a graph run, persist an application document, or
present controls. Concrete sources implement its provider contracts and the UI coordinates sessions.
