# Adaptive segment sizing: implementation status

The adaptive **target size is not enabled** for live partitioning or stealing,
and host history is not persisted. The pure target-size policy has unit tests;
its 1 MiB/s definition of "high goodput" and 25% EMA weight are provisional.

The task-local sizing state couples the target and **steal floor**. The
response-header latency EMA supplies baseline buckets of 2 MiB below 50 ms,
4 MiB from 50 to below 200 ms, and 8 MiB at 200 ms or above; before an
observation the legacy 1 MiB floor is used. When the target decreases, the
floor is capped at half the target; on growth it becomes one-quarter of the
target, capped at 8 MiB. Later latency bucket changes cannot exceed the most
recent coupling cap. All floors retain the absolute 1 MiB minimum, so a target
reduction can intentionally override the higher latency bucket.

HTTP 429/503 or a sharp drop in sampled committed goodput shrinks the target and
uses a 1 MiB floor for the next four steal decisions and at least a 12-second
cooldown. Target and floor growth are blocked during cooldown; repeated
congestion restarts it. Capacity inspection does not consume a decision. The
donor retains at least 1 MiB beyond its committed and in-flight prefix.
Response-header latency includes server and network delays and is not a direct
measurement of network RTT. Goodput sampling runs at roughly two-second
intervals; rate limiting, local disk or server stalls can be mistaken for
slowdown. State resets when the daemon reconstructs the map; no latency or
target history is persisted.

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

## Observability

The Network & splits detail view shows the current policy target, effective
steal floor and last change reason for a live split attempt. The target is
explicitly marked **not yet applied** to allocations. Policy updates use a
separate optional IPC stream, include the partial-file generation, and are
refreshed approximately every five seconds while active. A value change emits a
tracing line at `info` level. Metrics are live observations, not checkpointed or
persisted host preferences; disconnected or older daemons do not provide them.
