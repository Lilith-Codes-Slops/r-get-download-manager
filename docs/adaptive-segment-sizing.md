# Adaptive segment sizing: implementation status

**Not enabled in the live engine.** `split/adaptive_sizer.rs` contains a pure,
policy prototype with unit tests; the tests have not been run in this sandbox. It does not control initial partitions or
steals, collect live measurements, persist host history, or expose telemetry.
Existing transfers and v2 checkpoint layouts are unchanged. In particular, the
policy's 1 MiB/s definition of "high goodput" and 25% EMA weight are provisional
engineering choices, not measured optima.

## Integration constraints

- The existing map describes the **entire** file with at most 256 segment
  records. At an 8 MiB target, a file over 2 GiB already needs more than 256
  ranges; at the maximum 128 MiB target, this applies above 32 GiB. Splitting
  every new request to the target without a safe record-compaction strategy
  would exhaust the checkpoint limit and stall or require requests larger than
  the target. Recovery also limits sidecars to 64 KiB.
- Existing work stealing transfers the unwritten suffix of an active range,
  shortening the donor's boundary while excluding reserved in-flight writes.
  Applying a target to stealing must preserve that ownership barrier, keep
  checkpoint coverage intact, and retain any committed prefix hashes. It cannot
  simply change a range that contains committed or reserved bytes.
- Complete adjacent ranges have separate BLAKE3 prefix hashes. Merging records
  requires computing a hash for their combined bytes; concatenating digests is
  not equivalent. Compaction must snapshot identity and committed boundaries,
  hash only immutable completed ranges, validate again under the map lock, and
  preserve the old checkpoint until the new layout is durably committed.
  Recovery's existing coverage and hash checks must still pass for both layouts.
- HTTP response-header latency is a time-to-first-response proxy, not network
  RTT. Goodput must count successfully committed bytes, not downloaded bytes
  discarded after a steal; distinguish pauses, local bandwidth limiting and
  stalled transfers from genuine server slowdown. Do not let per-worker segment
  completions apply many consecutive growth changes in a burst without a
  deliberate cadence/hysteresis design.
- Host memory should use a clearly defined normalized host key (including a
  decision about ports/redirect destinations), be bounded, validate persisted
  sizes and buckets, and tolerate absent or corrupt optional history. Concurrent
  tasks must not accidentally overwrite newer measurements with stale ones.
  Storing it must not change v1/v2 checkpoint identity or block transfers on
  optional telemetry failure.

Before enabling this policy, add tests for compaction and reconciliation across
all checkpoint crash windows, in-flight steals, request-size caps, segment-count
exhaustion, 429/503, throttling, redirects, persistence and concurrent host tasks.
Retain the existing crash, pause, cancellation, checksum and single-stream tests.
