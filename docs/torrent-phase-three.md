# Torrent Phase 3: observation and picker limits

This is a design note, **not implemented functionality**. Phase 3 is blocked
on an operational single-file torrent path (Phase 1) and Phase 2 identity,
duplicate handling, removal and discovery groundwork. Fast resume is deferred.
No stall events, automatic recovery or torrent metrics are wired into the
daemon yet.

## Locked engine: librqbit 9.1.0

The following observations are from the `v9.1.0` source, not measurements of
this application. They need tests against the actual feature-enabled build and
must be reviewed if the lockfile changes.

- `ManagedTorrent::stats()` exposes total and progress bytes, completion,
  upload bytes, and an optional live snapshot. The live snapshot includes
  `downloaded_and_checked_pieces`, downloaded-and-checked bytes, fetched bytes
  and aggregate live peers. `progress_bytes` can also represent initialization
  checking progress: do not treat it as downloaded data until the torrent is
  actually live. A first-*piece* metric should use verified piece counts, not
  merely the first nonzero byte sample.
- `PeerConnectionOptions::max_request_window` sets a configured maximum. The
  engine also adjusts the window using the peer's extended handshake `reqq`.
  These are not an application-controlled, live per-peer BDP controller.
- The engine has internal streaming-related priority pieces. That is not a
  general public hook demonstrated to set first/last piece priority for ordinary
  torrent tasks; do not promise early preview or boundary boost on this basis.
- No application hook for a custom micro-endgame, duplicate-block policy or
  runtime per-peer queue sizing has been established. Do not add a parallel
  picker or request pipeline. Support for Fast Extension controls, request
  timeout counters, duplicate-byte counters, and safe per-peer dropping remains
  **unverified**, rather than being reported as supported or unsupported.
- Fetched bytes minus verified bytes is not an exact duplicate-byte counter:
  it can include in-flight and subsequently invalid data. If the exact metric
  is unavailable, report it as unknown rather than zero or a guess.

## Integration constraints

Once Phase 1 exists, observe monotonic time and verified piece/byte progress
only while the task is actively downloading and has peers. Emit one stall event
per continuous stall and a recovery event on renewed progress; pause, cancel,
restart and checking must not produce false stalls. Persisted clocks cannot
store `Instant`; restart must establish a new baseline. Saturate counters and
clamp duration calculations. Unknown peer or progress signals must stay unknown,
not silently become zero.

The current general progress IPC stream samples snapshots and can coalesce
intermediate states. A guaranteed stall/recovery event sequence requires an
ordered event mechanism or an explicitly documented best-effort contract.
A normal end-to-end download, induced stall, recovery, and no-peer/paused cases
must be tested before claiming the Phase 3 exit criteria.
