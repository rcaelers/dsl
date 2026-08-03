# `csv_word_writer`

## Responsibility

This module owns CSV serialization and persistence of runtime word streams.

## Boundaries

Output destination acquisition is injected through `OutputStorage`; UI file controls and platform
file access remain outside this processing node.
