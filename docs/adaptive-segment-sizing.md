# Adaptive segment sizing: implementation status

The adaptive target bounds successive HTTP requests and newly stolen work.
Split transfers can remember a last-good target and latency bucket per resolved
HTTP origin. The 1 MiB/s definition of "high goodput" and 25% EMA weight are
provisional.

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
slowdown. Recovered split tasks load the same optional host preference as new
split tasks; checkpoint data and identities do not include this preference.

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

## Host memory

The SQLite table `rget_host_sizing_v1` is keyed by the resolved response origin:
normalized scheme, host and effective port (not the original redirect URL or
full path). HTTP and HTTPS, and distinct ports, remain separate. The table is
bounded to 1,024 origins. A task uses a saved target of 1–128 MiB and one of
three latency buckets only after validating the row; absent, invalid, or
unreadable entries fall back to the normal 8 MiB target. A remembered bucket
seeds a representative response-header latency and its steal floor; the floor
is capped at half the remembered target (minimum 1 MiB). The first policy
snapshot uses reason `remembered` when history was loaded.

A value becomes eligible only after at least 12 seconds between successful,
fully committed bounded requests, with a fresh successful request, a measured
response latency and positive sampled goodput. Rejection, slowdown, a target
reduction, zero goodput, or pause resets this qualification. Storage is
best-effort, performed outside the ownership lock, no more than once every 15
seconds per task, and is not required for transfer success. Short transfers may
never qualify. Observation timestamps prevent older concurrent tasks from
replacing a newer saved sample; the implementation uses the system clock, so
large clock adjustments can affect ordering. Persisted latency is a coarse
bucket, not a measurement of network RTT.

## Observability

The Network & splits detail view shows the current policy target, effective
steal floor and last change reason for a live split attempt. Policy updates use a
separate optional IPC stream, include the partial-file generation, and are
refreshed approximately every five seconds while active. A value change emits a
tracing line at `info` level. Metrics are live observations; only eligible
last-good target and latency buckets are saved as optional host preferences.
Disconnected or older daemons do not provide live metrics.
