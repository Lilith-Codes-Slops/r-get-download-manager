# Changelog

All notable changes to this project are documented here. This file follows the
repository's pre-release status and does not imply that a tag or GitHub Release
has been published.

## [0.1.0] — unreleased

Status: PR #1 is open. Final verification at the exact current SHA is
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
- A global **Open containing folder** post-download action, configured in GUI
  Settings, stored by the daemon and applied to HTTP, single-file torrent and
  multi-file torrent completions.
- A redesigned `egui` GUI ported from the single-file browser mockup in
  `docs/gui-design/index.html`: ribbon, action bar, collapsible left rail,
  two-pane Tasks view, Downloads library, tabbed Settings and status bar.
- `state.md`, a living backlog recording wired controls, disabled controls and
  the long-term roadmap.

### Changed

- Pin `librqbit` to `=9.1.0`, matching the already-locked companion crates and
  the engine behavior documented in the Phase notes.
- Apply `rustfmt` across the workspace and store `DaemonEvent::ProgressUpdate`'s
  `DownloadInfo` behind `Box`, so `-D warnings` does not reject the large enum
  variant. The serialized IPC event shape is unchanged.
- Replace the disabled Post-download actions placeholder with a read-only
  status view and a link to the Settings control.
- Replace the previous basic GUI layout with the ported design; keep every
  daemon-unready control visible but disabled with `TODO(state.md)` comments.
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
- A formatting/Clippy fix commit addressed the previously reported `rustfmt`
  diffs and `large_enum_variant` error at `07fd541`.
- The post-download action feature commit and the subsequent GUI port commit each
  changed the SHA again. The full matrix, including `cargo fmt --check`,
  `cargo clippy -- -D warnings`, the Cargo test suites and the release build,
  must still be rerun at the new final SHA. See `docs/finalization.md` and
  `state.md`.

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
- Windows daemon support, Metalink, mirrors, extraction, scheduling, file
  mutations, automatic checksums, unimplemented post-download actions (close
  rGet, antivirus scans, system power actions) and a daemon-wide
  graceful-shutdown barrier remain unimplemented. GUI rate limiting is now wired
  as a global `SetLimit` control; the global "open containing folder" action is
  implemented, other automation is not.
- GUI-only preferences (theme, compact mode, alerts, categories) are
  session-scoped. Upload/seeding, RSS, labels, favorites, editable categories,
  magnets, sounds, native file picker and file mutations are disabled in the
  ported GUI and tracked in `state.md`.
- The earlier CachyOS single-file public run was a smoke test only and does not
  satisfy the multi-file acceptance criterion.
