# `packet_framer`

## Responsibility

This module owns framing configured event or word inputs into protocol-packet runtime values.

## Boundaries

It does not decide packet display labels, viewer spans, or protocol renderer registration. The
matching graph-node feature owns that presentation metadata.
