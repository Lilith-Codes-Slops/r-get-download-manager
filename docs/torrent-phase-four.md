# Torrent Phase 4: concurrency groundwork

This is an **incomplete Phase 4 implementation**. The daemon now has a FIFO
limit on active torrent downloads, but upload, seeding and independent
file-check/seed limits are not implemented. Do not treat the presence of a
stored seed flag as enabling seeding. Fast resume remains deferred.

## Active download queue

The default limit is **3** active torrent attempts, including metadata lookup,
download, session shutdown and publication. Extra tasks are `queued`; their
one-based waiting position is reported in the connection note (`dl list` and
GUI details). When a permit is released, the next registered task can start.
Queued tasks can be canceled or removed; cancellation does not start their
session. Limits are process-local and apply to torrent tasks, not HTTP tasks.

To change the limit before starting the daemon, set an integer from 1 to 32:

```bash
export RGET_TORRENT_MAX_ACTIVE=1
```

Invalid values fall back to 3. This environment override is **not** persisted
as a setting and must be present in the daemon's own environment. Already
running daemons must be stopped before changing it. On daemon restart, waiting
and active torrent tasks are marked interrupted rather than automatically
requeued; retry from scratch follows the existing retained-stage rules. The
queue is in memory; its order is not durable across restarts.

## Upload and seed limitations

The current session configuration explicitly disables upload and stops after
completion. Seed toggles, upload caps, seed resumption, file-check limits, seed
limits and global peer ceilings are **not implemented**. No upload behavior was
changed by the queue work.

In locked librqbit 9.1.0, `Session::ratelimits` controls that session's upload
rate and `AddTorrentOptions::ratelimits` supplies a torrent-level rate. rGet
currently creates a separate session for each torrent; setting the same limit
on each is **not a global cap**. A zero/absent librqbit rate means unlimited,
not disabled. The `disable_upload` setting is fixed into a session/torrent at
creation, so it cannot serve as a live on/off control. A safe shared limiter or
shared-session design, retained metainfo and validated file ownership are still
needed before enabling seed commands or changing the default upload policy.
The current code does not implement or claim focused unchoke allocation.

## Verification still required

The user reported the queue and cancellation unit tests passing with both
workspace and no-default-feature test runs. A live queue test is still needed.
After running the workspace tests, verify manually in an isolated lab with `RGET_TORRENT_MAX_ACTIVE=1`:
start a deliberately slow authorized torrent, add another, confirm the second
stays queued, and cancel or finish the first. Confirm the second starts. Repeat
by canceling the queued task and ensure no session or partial is created for
it. Record results; do not infer a pass from the unit tests alone. Public-swarm,
upload and seed behavior remain unverified.

## Settings groundwork and upload decision

Phase 4 settings have a versioned SQLite row, defaults and validation, but no
user-facing settings control yet. The loaded settings are logged as **not
applied**. In particular, the active download queue still uses its existing
default and `RGET_TORRENT_MAX_ACTIVE`, not the stored limit. Upload remains
engine-disabled, regardless of the stored upload switch or caps. The check
and seed limits are not enforced. The settings storage and startup logging
have not been compiled or tested in this environment.

The locked engine's upload scheduler calls its own torrent and session
limiters; it has no public external payload-permit hook. A missing rate limit
means unlimited, not zero, and replacing the rate limiter does not revoke
previously acquired/in-flight work. The user chose to defer the shared upload
architecture decision. Do not enable upload or claim a global cap until a
separate safe design has been selected and verified.
