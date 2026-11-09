# Torrent Phase 1: approved v1 scope and status

**Initial daemon integration is present but not yet live-verified.** On Linux,
torrent support is enabled by default. A magnet or local `.torrent` file is
dispatched to an isolated v1 librqbit session; completion is published using
no-clobber naming after the session is stopped. An 8 MiB cap applies to local
metainfo files, with a 60-second metadata timeout. The normal HTTP path remains
separate. The integration test uses a local tracker and seeder, but its result
must be confirmed by running it; it is not evidence of a public-swarm test.

The approved Phase 1 uses the existing, locked librqbit 9.1.0 engine with
these explicit limitations:

- v1 metadata and v1 piece validation only; reject unsupported v2-only input.
  Do not advertise v2 block verification. Hybrid support must be tested and is
  not assumed.
- Use librqbit's picker and synchronous storage defaults. It writes incoming
  blocks before validating each completed piece; it does **not** provide the
  proposed whole-piece-in-memory, verify-before-write or off-reactor guarantees.
  Treat all partial data as untrusted until the engine validates it.
- Enable librqbit's `disable-upload` feature and configure it on every session,
  and stop the session before publication. The integration configures both;
  zero upload and stop-on-complete still require live verification.
- Local discovery, webseeds, multiple files and seeding remain out of scope.
  Public-torrent/manual-network verification remains outstanding.

Outstanding work includes live verification, stronger transition delivery,
proper torrent-specific task controls and rate limiting, and restart recovery. The
existing progress stream is sampled and may coalesce intermediate transitions;
an explicit ordered transition stream is needed if every transition must be
observable. URL-keyed HTTP tasks and recovery must not accidentally interpret
torrent paths as HTTP checkpoints. The CLI resolves relative `.torrent` paths
before sending them to a daemon with a different working directory.

Feature-specific tests should be run as well as the normal workspace suite:

```bash
cargo test --workspace --locked
cargo test -p downloader-core --features torrent --locked
```

## Phase 2 fast-resume decision

Fast resume is deferred. The locked librqbit 9.1.0 implementation samples
claimed pieces and its sampled check appears to test only whether `check_piece`
returned an error, not whether it returned `Ok(false)` for a hash mismatch.
Sampling also cannot prove all unsampled data is intact. Do not enable
`fastresume` until its integrity behavior and crash-safety are resolved and
tested. This does not block the remaining Phase 1 download integration.

The future observation and picker scope is recorded in [torrent-phase-three.md](torrent-phase-three.md); it is not implemented yet.

## Test and current limitations

```bash
cargo test --workspace --locked
cargo test -p downloader-core --no-default-features --locked
```

The first command includes `torrent_download_ipc`, which launches a local test
tracker, test seeder and isolated daemon; it does not contact a public swarm for
test data. The second checks that HTTP works without the optional torrent
feature. For a manual test, build and use only a public, authorized single-file
v1 torrent and an isolated `HOME`/`XDG_RUNTIME_DIR`/`XDG_STATE_HOME` as in
[manual-live-verification.md](manual-live-verification.md). Add a quoted magnet
or an absolute `.torrent` path using `dl add`, then use `dl list` repeatedly to
observe the current state, bytes, rate and peer note. Compare the published
file's digest with a trusted independent digest. Record results rather than
assuming success. Avoid magnet links for private torrents: metadata isn't known
until after peer discovery.

Torrent staging directories are private `.rget-torrent-*` directories under
Downloads. Failed/canceled/interrupted transfers may leave them intact for
inspection; they are not automatically resumed or removed by task deletion.
No stage should be deleted based solely on a displayed path. A restart marks
incomplete torrents failed and never feeds them into HTTP checkpoint recovery.
Retry starts from scratch. Phase 2 will address identity, cleanup and recovery.

Torrent pause/resume, torrent speed limiting and live connection changes are not
implemented. The current connection value is passed as the maximum peer count
for a newly started torrent; it is not an HTTP split-worker count. HTTP speed
limits do not limit torrent traffic. Duplicate adds by the same source reuse an
active task, but info-hash deduplication is not implemented. General progress
updates are sampled, so intermediate torrent states can be coalesced rather
than all being visible. Tracker/DHT behavior, rejection and re-request of a
corrupt piece, zero upload and real public-swarm completion remain unverified.
