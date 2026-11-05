# XP4 JPEG transport

This crate models a register-driven encoder with explicit caller-supplied
encoding assumptions. Without an encoding law, START refuses. The admitted
subset uses packed Y0/Y1/Cb/Cr pairs, uploaded quantizers, declared restart
spacing, SOI-to-EOI output and a positive deterministic completion delay.
These choices are model inputs, not a claim of measured hardware fidelity.

Row stride is independent of image width. Complete source rows, including
padding, must fit the declared bounded span; padding is never sampled. Source
bytes are captured at START and destination writes occur at the deadline.
Addresses, alignment, overlap, output capacity and arithmetic overflow are
checked before model DMA. Completion updates the modeled status/count and IRQ;
unsupported configurations refuse.

Tests exercise padded rows, encoding/decoding, interrupts and acknowledgment,
output overflow, reset, unsupported configuration and DMA bounds. Board wiring
is optional and tested separately. This does not supply stock capture control,
card-file persistence, a firmware menu or a complete camera emulator.
