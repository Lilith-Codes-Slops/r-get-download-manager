# Adaptive segment sizing: implementation status

The adaptive target bounds successive HTTP requests and newly stolen work.
Host history is not yet persisted. The pure target-size policy has unit tests;
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

## Allocation and recovery

The checkpoint map continues to cover the whole file with at most 256 logical
records. Each owner requests `min(remaining, target)` bytes at a time inside
its existing record, so a large file does not need one checkpoint record per
HTTP request. The complete HTTP request is leased under the ownership lock;
stealing can transfer only the unallocated suffix beyond the lease, capped by
the current target and subject to the floor and donor minimum. The boundary of
a logical segment may therefore shorten, but no in-flight request changes.
The target can change during a request; the next request uses the new value.
Checkpoint records still contain committed prefixes and their existing hashes;
leases are transient and reconstructed after recovery.

Host memory remains future work. It needs a normalized host/port key, validated
and bounded stored values, ordering for concurrent tasks, and best-effort
persistence that does not affect transfers or checkpoint identity.

## Observability

The Network & splits detail view shows the current policy target, effective
steal floor and last change reason for a live split attempt. Policy updates use a
separate optional IPC stream, include the partial-file generation, and are
refreshed approximately every five seconds while active. A value change emits a
tracing line at `info` level. Metrics are live observations, not checkpointed or
persisted host preferences; disconnected or older daemons do not provide them.
