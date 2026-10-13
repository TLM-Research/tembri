# ADR: Tembri workspace boundary and CLI name

- Date: 2026-10-10
- Status: accepted

## Context

The previous Rust measurement workspace lived directly in the tlm_proj directory.
That directory also contains separate TLM research trees and miscellaneous
research material. The toolbox now has an Tembri repository identity and
needs an independently understandable project boundary.

## Decision

Create the standalone workspace at:

    /home/dguo/dev/tlm_proj/tembri

Move the Tembri workspace manifest, Rust crates, binary, configuration,
ADRs, measurement data, historical reports, and measurement specifications into
that directory. Keep the TLM, TLM-Private, and TLM-Research directories outside
the Tembri workspace.

Rename the user-facing Cargo package and executable from tlm to tembri.
The binary source now lives in bin/tembri. Internal crate names retain
their existing tlm-* names for now because they are implementation details and
changing them has no user-facing benefit.

## Consequences

- Running Cargo commands from the Tembri root keeps relative data and
  configuration paths self-contained.
- Reports continue to use the shared /share/tlm_proj/report output location
  where configured, so existing report-delivery workflows are preserved.
- The new public command form is:

    cargo run -p tembri -- <command>

- TLM research files remain available but cannot be mistaken for part of the
  executable measurement toolbox.
