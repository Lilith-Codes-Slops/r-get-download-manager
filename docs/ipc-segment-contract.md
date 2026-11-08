# Live connection and segment IPC contract

This records the current Linux daemon/client wire contract. Preserve these shapes
and meanings when extending it; add optional, defaulted fields rather than
repurposing existing ones. Frames are a four-byte little-endian length followed
by UTF-8 JSON. Requests use the `op` tag; responses and events use `event`.
`task_id` currently identifies the task by its original URL.

## Connection control

Request: `{"op":"set_connections","task_id":"<URL>","count":8}`.
`count` is an integer from 1 through 32 and is the requested worker ceiling,
not a socket count. A successful, persisted request returns
`{"event":"connections_changed","task_id":"<URL>","requested":8}`;
invalid counts, missing tasks and unsupported states return `task_failed`.
If persistence fails after the operation was applied, `task_failed` says so;
clients must not assume the change was rolled back. The GUI updates requested
count on acknowledgment and queue refreshes remain the source of task state.

Only existing split tasks in downloading or paused state accept this request.
Single-stream-to-split conversion is **not implemented**, including when a
single-stream task has a valid v1 checkpoint. A split task can be reduced to
one worker and increased again. Decreasing allows selected workers to finish
their current segments; a changed ceiling is forwarded asynchronously. Actual
concurrency can be lower because of available work, adaptation or rejection.
`active_connections` denotes workers owning a range (or a single-stream
transfer), not TCP sockets. It is a point-in-time observation and can change
before a client displays it.

## Segment stream

A separate connection sends `{"op":"subscribe_segments"}` and receives
multiple framed events. It begins with `segments_reset` followed by zero or
more `segments_snapshot` events, and subsequently delivers snapshots, deltas,
removals and `segments_heartbeat` events. A subscriber that disconnects or
falls behind must reconnect and rebuild from the new reset and snapshots.
Older daemons may not support this request; the GUI falls back to queue samples.

A snapshot has `task_id`, `generation` and `segments`. Each segment has numeric
`segment_id`, `start_byte`, `end_byte`, `downloaded`, and `status` (`pending`,
`active`, or `done`). Ranges are half-open `[start_byte, end_byte)` and
`downloaded` counts committed writes from the start of that range. A snapshot
replaces the entire task layout; steals can change boundaries and add segments.
The `generation` is the task's partial-file name and prevents events from an old
attempt being applied to a retry. Segment IDs are stable only within an active
layout/generation; clients must accept a replacement layout after recovery.

A delta has this shape:

```json
{"event":"segment_update","task_id":"<URL>","segment_id":"0","start_byte":0,"end_byte":1048576,"downloaded":65536,"mirror_id":null,"generation":".rget-1-0.part","status":"active"}
```

The delta's `segment_id` is a string, unlike the numeric ID in a snapshot.
`generation` and `status` are optional for compatibility with older event
producers; this GUI only applies deltas with both fields and matching generation
and range. `segments_removed` carries `task_id` and `generation`; it removes
only that matching attempt. Committed progress in events is not necessarily a
durable checkpoint. A crash can cause uncheckpointed data to be downloaded
again. Event delivery is not a durable log and may be delayed or coalesced.

## Optional policy stream

`{"op":"subscribe_segment_policies"}` opens a **separate** framed event
stream; the existing segment stream is unchanged so older subscribers do not
receive unknown event variants. It begins with `segment_policies_reset`, then
zero or more current `segment_policy_update` events; changes are sent as they
occur and unchanged values are refreshed approximately every five seconds
while a split transfer is active. `segments_heartbeat` is also used on this
stream. A lagging subscriber must reconnect and rebuild after the reset.

An update is shaped as:

```json
{"event":"segment_policy_update","task_id":"<URL>","generation":".rget-1-0.part","target_size":8388608,"steal_floor":2097152,"reason":"rtt"}
```

Sizes are bytes. The target is currently a calculated policy value **not used
for live segment allocation**; the steal floor is applied to new steals. Reason
is a string such as `initial`, `goodput`, `rtt`, `rejection`, or `slowdown`;
clients should tolerate future values. `segment_policy_removed` carries the
same `task_id` and `generation`. Values are per attempt, not persisted host
history. Ignore an event whose generation does not match the current task's
partial name. If this optional stream is unsupported or disconnects, clear the
cached policy instead of displaying stale values.

## Destination fields in queue and state responses

`GetDetailedQueue` includes an optional `destination_path` alongside progress.
For an unfinished task this is a **planned** destination, derived from its
persisted filename; it can change if another file wins a publication race.
`final_path` remains reserved for a file actually published by the task. A
single-task `GetSnapshot` and task control state replies may also include
`destination_path`; the initial Add reply can omit it while the server response
is pending. The new fields are optional when reading older daemons. `SpeedTick`
has an optional destination field for compatibility, but this daemon does not
currently publish a general SpeedTick subscription; progress is polled via the
structured queue. Planned destinations must never be treated as owned files
for deletion.
