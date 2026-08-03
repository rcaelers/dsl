# `signal_processing::logic_analyzer`

## Responsibility

This module owns driver-neutral logic-analyser source, trigger, and capture-configuration
contracts consumed by concrete device processing nodes.

## Boundaries

It does not select a USB implementation, define a concrete device model, render configuration, or
persist graph-node state. Device-specific validation and transport remain in processing and platform
owners.
