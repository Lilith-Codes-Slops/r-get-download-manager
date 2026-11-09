# Torrent Phase 2 groundwork

The following implementation is newly added and **not yet verified by running
the tests or live network checks** in this environment. Fast resume remains
disabled; interrupted downloads are not automatically resumed.

## Identity and duplicates

New torrent tasks use a generated stable IPC key, retained in `DownloadInfo.url`
for compatibility with existing queue and control messages. The optional
`torrent` record holds the original source, v1 info-hash, creation timestamp,
owned staging name and seed flag; it is stored as JSON in the existing SQLite
queue. The seed flag defaults to false and is persisted, but there is no user
control and it does not enable seeding. Existing HTTP tasks and old queue records
without this field remain readable.

For v1 magnets the info-hash is known immediately; local metainfo is read once
(up to 8 MiB) and the raw info dictionary is hashed before creating the task.
The captured bytes are used for the transfer, preventing a changed file from
invalidating the duplicate decision. Under the queue lock, an existing hash
returns its existing task ID rather than creating another task. The GUI focuses
that task and CLI uses the returned ID. Malformed sources may fail immediately.
Legacy torrent records lacking the new field are recognized by identical source
only; cross-source info-hash migration for those records is not implemented.

A failed/canceled task with a retained stage cannot be retried directly: first
remove it with keep or delete, then add it again. This avoids silently orphaning
or deleting its previously owned partial. Removing with keep leaves the partial
on disk and deletes the queue record; no resume data exists yet. Removing with
delete checks the recorded staging directory and task-specific ownership marker,
and refuses unrecognized files or symlinks rather than deleting them. Orphaned
or unrecorded stages are never automatically adopted or removed. The existing
`RemoveTask { delete_file: bool }` remains the IPC operation; cancellation of a
metadata wait uses the existing `Cancel` command.

## Discovery

Each active info-hash has its own librqbit DHT cache under
`$XDG_STATE_HOME/rget/torrent-dht/<hash>/dht.json` (or the matching default
state directory). Per-hash directories avoid concurrent sessions clobbering
librqbit's process-scoped temporary filename. The library periodically writes
its routing table and reloads it when that hash's next session starts; corrupt
JSON is ignored by the library. It pings questionable/unknown routing nodes as
part of its own maintenance. This is **not** a single daemon-wide cache,
startup prewarm, guaranteed immediate ping of every saved node, or proof of
fast cold discovery. Short sessions can end before the first periodic write.

Local peer discovery is enabled through librqbit with a listening TCP socket;
if multicast initialization fails, librqbit logs a warning and continues. If
persistent session setup fails, the daemon logs a warning and falls back to
nonpersistent discovery with local discovery disabled. Actual LAN discovery
still needs a two-host test.

The locked engine uses concurrent tracker futures and merges tracker and DHT
peer streams; no independent tracker implementation or tier-racing guarantee is
added here. Upload remains disabled. Actual tracker/DHT timing and zero upload
need live verification. A shared-session redesign is still needed for a global
DHT warm cache and startup lifecycle.
