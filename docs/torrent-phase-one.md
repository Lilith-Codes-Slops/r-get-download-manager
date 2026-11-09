# Torrent Phase 1: approved v1 scope and status

**Not yet wired into the daemon.** The `torrent` feature currently contains
source classification, lifecycle definitions, session defaults and a list-only metadata resolver with a 60-second timeout
and an 8 MiB limit for local metainfo files. It does not yet accept or download torrents. Do not use it as a torrent client.

The approved Phase 1 uses the existing, locked librqbit 9.1.0 engine with
these explicit limitations:

- v1 metadata and v1 piece validation only; reject unsupported v2-only input.
  Do not advertise v2 block verification. Hybrid support must be tested and is
  not assumed.
- Use librqbit's picker and synchronous storage defaults. It writes incoming
  blocks before validating each completed piece; it does **not** provide the
  proposed whole-piece-in-memory, verify-before-write or off-reactor guarantees.
  Treat all partial data as untrusted until the engine validates it.
- Enable librqbit's `disable-upload` feature and configure it on every session;
  also explicitly stop/remove the torrent after completion. Neither upload
  prohibition nor cleanup is implemented in the daemon yet.
- Local discovery, webseeds, multiple files and seeding remain out of scope.
  Public-torrent/manual-network verification remains outstanding.

Remaining integration work includes wiring and testing metadata resolution,
isolated owned temporary storage, no-clobber atomic publication,
full lifecycle and peer telemetry in the existing queue/event stream, task
controls, restart behavior and explicit cleanup, and integration tests. The
existing progress stream is sampled and may coalesce intermediate transitions;
an explicit ordered transition stream is needed if every transition must be
observable. URL-keyed HTTP tasks and recovery must not accidentally interpret
torrent paths as HTTP checkpoints. Relative paths passed by CLI need to be
resolved before sending them to a daemon with a different working directory.

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
