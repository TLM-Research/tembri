# ADR: public mempool lifecycle measurement

- Date: 2026-10-10
- Status: accepted

## Context

The toolbox previously retained only a transaction hash and first-public-seen
timestamp. That was sufficient for a simple public/private visibility label, but
not for measuring the delay to public visibility, public-mempool residence time,
canonical admission, or transactions abandoned after unsuccessful admission.

A public observer cannot generally observe a wallet's first private submission.
The design must therefore avoid presenting public observation time as sender
submission time.

## Decision

Add a pure Rust lifecycle module in tlm-classify and shared serializable records
in tlm-model.

Inputs are newline-delimited JSON records:

- MempoolObservation: hash, sender, nonce, seen_at_ms, and optional
  submitted_at_ms.
- MempoolAdmission: hash, admitted_at_ms, and block_number.

The lifecycle calculation canonicalizes hashes and sender addresses, retains the
first and last public observation for each hash, and joins canonical admissions
by hash.

Each observed hash is assigned one of three outcomes:

1. Admitted: a canonical admission record exists. Residence and admission
   delay are measured from first public observation to admission.
2. DroppedBySender: no admission exists and a later, distinct hash with the
   same sender and nonce was publicly observed. The replacement observation
   closes the observed residence interval.
3. PendingOrUnobserved: neither admission nor a replacement signal exists.
   Its duration ends at the last public observation and is not a claim that the
   transaction has left the mempool.

The CLI command is:

    tembri mempool-lifecycle --observations OBSERVATIONS.ndjson
      --admissions ADMISSIONS.ndjson --out REPORT.md

It writes a Markdown aggregate report and a detailed lifecycle NDJSON file.

## Consequences

- The metric is explicit about its observation boundary: all durations begin at
  first public observation.
- First-public-seen delay is only emitted when submitted_at_ms is supplied.
  It remains unavailable for ordinary public-mempool archives.
- Sender drops are conservative inferences, not direct cancellation evidence.
  They require sender and nonce data and do not conflate a low fee with a
  sender decision.
- The pure calculation is unit-tested for admission timing, replacement-based
  drops, and the precedence of canonical admission over a later replacement
  observation.
- Future ingestion adapters can normalize provider-specific feeds into these
  two records without changing lifecycle semantics.
