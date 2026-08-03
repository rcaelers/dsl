# `parallel_decoder`

## Responsibility

This module owns parallel-bus sampling, assembly, and decoded-word production for its configured
clock, data, and qualifier inputs.

## Boundaries

It does not own graph socket definitions, display formatting, or host execution selection. Those
concerns remain with graph-node, UI, and platform owners.
