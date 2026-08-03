# `text_file_writer`

## Responsibility

This module owns line-oriented text persistence from runtime text streams.

## Boundaries

It receives destination storage through `OutputStorage` and does not own graph UI, dialog behavior,
or host-specific output APIs.
