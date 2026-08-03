# `dslogic_u3pro16`

## Responsibility

This module owns the DSLogic U3Pro16 capture protocol, source state machine, and processing-source
behavior.

## Boundaries

USB transport and FPGA-image acquisition are host capabilities injected by platform composition.
Graph state, editor controls, and target selection do not belong to this module.
