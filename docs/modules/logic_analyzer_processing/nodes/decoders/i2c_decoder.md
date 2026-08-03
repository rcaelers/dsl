# `i2c_decoder`

## Responsibility

This module owns I²C signal decoding and I²C packet production from its configured processing
inputs.

## Boundaries

It does not own I²C graph sockets, packet rendering, saved-node migration, or UI panels; those
belong to the matching graph-node feature.
