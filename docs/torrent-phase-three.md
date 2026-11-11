# Torrent Phase 3: observation and picker limits

Phase 3 observation is implemented but **not yet live-verified**. It runs on
the existing single-file torrent path; fast resume remains deferred. The code
reports stalls and subsequent observed data, but does not drop peers or force
requests. There is no custom picker or BDP pipeline.

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

The daemon samples about every 500 ms. Once a live torrent has at least one
peer and has received payload, 30 seconds without an increase in librqbit's
accepted fetched-byte counter emits one stall event. This counter can include
bytes later rejected by piece verification, so it is a network-activity signal,
not verified progress or a duplicate-byte count. No peers, paused/checking,
missing counters, and counter resets reset the baseline. Losing peers closes
the measured stall; a later eligible sample with new bytes emits recovery.
Cancel/failure does not emit a fabricated recovery. No automatic peer drop or
re-request is attempted.

Per-attempt metrics include metadata resolution time, sampled time from metadata
resolution to the first verified piece, sampled first-piece-to-finish time,
stall count and durations. A single sample may see both first piece and finish,
producing a zero interval. Duplicate bytes, request timeouts and peers dropped
are `null` when unavailable; they are not guessed. Completed metrics are stored
in the queue. A restart clears the transient stalled flag and current duration;
`Instant` values are never persisted. Counts and durations saturate.

`DownloadInfo.torrent.metrics` appears in the existing queue/progress stream;
GUI details and `dl list` show a subset. The general stream can coalesce
snapshots. `SubscribeTorrentEvents` is a separate opt-in, bounded transition
stream: after `TorrentEventsReady`, it sends `TorrentStalled`,
`TorrentRecovered`, and heartbeats. Events are ordered for a connected consumer,
not replayed or persisted; a slow/lagging consumer is disconnected. On
reconnect, inspect the current queue for current metrics. Older clients do not
receive new event variants unless they opt in.

To watch future events in a separate terminal:

```bash
./target/debug/dl torrent-events
```

For a manual check, use an isolated lab and an authorized torrent from a
controlled local seeder. Run with `RUST_LOG=info` to see capability logging.
Ensure exactly one connected peer, start `dl torrent-events`, and arrange for
that test peer to stop delivering payload for more than 30 seconds while the
connection remains live. Resume the peer and observe the event stream and
`dl list`. If the peer disconnects, another peer supplies data, or the transfer
finishes too soon, the stall check is inconclusive—not a pass. Also observe
that no-peer and initialization/checking periods do not create a stall. Torrent
pause is not currently exposed. These live
checks, corrupt-piece behavior, and public-swarm throughput remain unverified.
