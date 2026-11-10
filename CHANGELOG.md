# Changelog

All notable changes to this project are documented here. This file follows the
repository's pre-release status and does not imply that a tag or GitHub Release
has been published.

## [0.1.0] — unreleased

Status: PR #1 is open. Final verification at the exact post-fix SHA is
pending, and no merge, tag, or GitHub Release has been created.

### Added

- HTTP/HTTPS download controls, collision handling, cancellation, task removal,
  rate limits, progress events and SQLite-backed single-stream recovery.
- Experimental fixed-split downloads with range validation, adaptive workers,
  work stealing, checkpointing, startup recovery and no-clobber publication.
- Local BitTorrent v1 lifecycle on Linux GNU: metadata preflight, owned
  staging, gated storage, pre-publication piece verification, exact no-replace
  directory publication, durable identity, conservative recovery and journaled
  removal.
- GUI root/file-count display for torrent tasks and opt-in public-network
  torrent acceptance tests.
- Dual-license texts, `.gitignore`, `rust-toolchain.toml`, finalization
  checklist and refreshed release documentation.

### Changed

- Pin `librqbit` to `=9.1.0`, matching the already-locked companion crates and
  the engine behavior documented in the Phase notes.
- Apply `rustfmt` across the workspace and store `DaemonEvent::ProgressUpdate`'s
  `DownloadInfo` behind `Box`, so `-D warnings` does not reject the large enum
  variant. The serialized IPC event shape is unchanged.
- Clarify Linux-only daemon support and the Linux GNU torrent target in
  `README.md`.
- Mark `docs/torrent-phase-five.md` as an append-only engineering log and add a
  finalization status section.

### Validation

- The user reports that the deterministic Cargo suites and the public v1
  multi-file torrent acceptance test passed on their PC.
- A verbatim final public-run log was not attached; this branch records a
  **user-reported pass**, not an assistant-observed one.
- Earlier user-confirmed results at `251cb71`: workspace suite green
  (206 core unit tests plus integration/GUI/IPC suites), no-default-features
  suite green (96 core unit tests), and fault suite green (10/10, 42.12s).
- At `70c56e5`, the public-test guard suite reported 2 passed and 2 ignored.
- A formatting/Clippy fix commit addresses the previously reported `rustfmt`
  diffs and `large_enum_variant` error. The full matrix, including
  `cargo fmt --check`, `cargo clippy -- -D warnings` and the release build, must
  still be rerun at the new final SHA. See `docs/finalization.md`.

### Known limitations

- Torrent ingest supports local ordinary v1 `.torrent` files only. Multi-file
  magnets, v2/hybrid/private torrents, padding/symlink/special files, webseed
  retrieval, upload, seeding, fast resume and applied file priorities are out
  of scope.
- `Session::stop()` does not join engine tasks; the storage gate provides
  storage quiescence only. `quiescence-unknown` blocks removal until restart.
- The multi-file public test uses the project's recovery verifier, not an
  independent implementation.
- Cleanup assumes exclusive control of the private staging tree and is not
  hostile-same-user concurrency safe.
- Windows daemon support, Metalink, mirrors, extraction, scheduling, GUI rate
  limiting, file mutations, post-download automation, automatic checksums and
  a daemon-wide graceful-shutdown barrier remain unimplemented.
- The earlier CachyOS single-file public run was a smoke test only and does not
  satisfy the multi-file acceptance criterion.
