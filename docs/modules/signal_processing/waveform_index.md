# `signal_processing::waveform_index`

## Responsibility

This module owns finite and growing generic waveform indexes plus bounded sampled-window queries.

## Boundaries

Capture sources provide packed samples and identities. The module does not acquire files, choose
storage locations, render waveforms, or decide which graph source is visible.
