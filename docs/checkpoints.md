# Checkpoint and recovery formats

This describes the Linux daemon's current implementation, not a stable external
API. Do not hand-edit checkpoint files or database records. Both formats store a
JSON sidecar next to a task-owned partial file named like `.rget-123-0.part`: the sidecar is the partial name plus `.meta`. The queue and copies
of checkpoint records are stored in `$XDG_STATE_HOME/rget/queue.sqlite3`, or
`$HOME/.local/state/rget/queue.sqlite3` when `XDG_STATE_HOME` is unset. Records
are keyed by the task URL. An orphan partial is not adopted solely because a
sidecar exists.

## Version 1: single stream

The sidecar and `rget_checkpoints_v1` record have `version: 1`, a positive
`sequence`, `task_url`, `resolved_url`, `filename`, `partial_name`, a strong
`etag`, `total`, `downloaded`, and `prefix_blake3`. `downloaded` counts the
contiguous prefix `[0, downloaded)`; `prefix_blake3` is its hexadecimal BLAKE3
hash, including the empty prefix when nothing has been downloaded.

## Version 2: split transfer

The sidecar and `rget_split_checkpoints_v2` record have `version: 2`, the same
identity fields, `requested_connections` (2–32), and `segments`. A segment has
`start`, `end`, `downloaded`, and `hash`. Its byte range is half-open
`[start, end)`; its hash covers only `[start, start + downloaded)`. The sorted
ranges must be contiguous, non-overlapping, and cover `[0, total)`. There may
be at most 256 segments. Stealing can change segment boundaries between
checkpoints; ownership and in-flight write reservations are not serialized.

## Durability and reconciliation

* A checkpoint syncs file data, atomically replaces the JSON sidecar, syncs its
  directory, and stores the record and task state in a SQLite transaction.
  Version 2 first snapshots completed writes under the segment-map lock; it
  does not count writes still in flight. Workers can continue after the snapshot.
* On startup, a task must identify the partial and the sidecar. Both sidecar
  formats are limited to 64 KiB. Filenames, URLs, strong ETags, bounds and
  metadata are checked, and partials/sidecars are opened without following
  symlinks on Linux. Unsafe, missing, inconsistent or corrupt metadata fails
  recovery rather than assuming bytes are valid.
* A matching database copy is optional, but when present it must match the
  checkpoint identity. Equal sequence numbers must have identical contents.
  Otherwise the newer sequence is chosen only when verified progress has not
  regressed. For v2, the newer segment prefixes must cover all bytes verified by
  the older layout, and both snapshots are hash-checked against the file.
  V1 verifies the sidecar prefix and the selected prefix.
* V1 truncates uncheckpointed bytes past its verified prefix. V2 does **not**
  truncate individual ranges: uncheckpointed tails remain in the shared file
  and are overwritten when those ranges resume. The resumed requests must
  still match the saved strong ETag and resolved URL; range responses are
  validated before their bodies are written.
* Paused and pausing tasks are restored paused. A task recorded as canceling is
  not restarted. After publication, a final file is accepted as complete on
  recovery only when a complete database checkpoint verifies its content; file
  size alone is not proof. Otherwise a destination collision is not overwritten.

A checkpoint hash verifies locally recorded bytes, not a publisher-provided
whole-file checksum. Checkpoints and daemon-kill tests do not prove all possible
power-loss, disk-error, or shutdown interleavings; the daemon-wide graceful
shutdown barrier remains outstanding.
