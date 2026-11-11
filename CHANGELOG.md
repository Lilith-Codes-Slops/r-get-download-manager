# Changelog

All notable changes to this project are documented here. This file follows the
repository's pre-release status and does not imply that a tag or GitHub Release
has been published.

## [0.1.0] — unreleased

Status: PR #1 was squash-merged as
`99c845fd3f496008b2a585ae7592671f332eba68` after a green nine-step matrix at
`6cc0baf…`, PR #2 as `524a35b8fd5660970c98af0b9de302e36c12c390` after a green
six-step matrix at `8bd754e…`, and PR #3 as
`c9e8a8bf6398bc29e1ff945ed73e70221e42c0a4` after the docs-only six-step matrix
was green at `1c26bf9…` (PR #3's body is the authoritative record). The active
branch `arena/b93940a1-r-get-download-manager` adds `.github/workflows/ci.yml`
first; no tag, GitHub Release, checksums or `--auto` exist.

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
- Local GUI preference persistence in `gui-preferences.json` under
  `$XDG_CONFIG_HOME/rget/` or `~/.config/rget/`: theme, compact mode,
  confirm-before-delete, in-app alerts, rail pinning, sort field/direction and
  the active Settings tab are restored on startup, with a versioned and
  tolerant JSON round-trip plus unit tests.
- A `Ctrl+K` command palette over existing wired actions only, with
  case-insensitive label/shortcut/keyword search, arrow/Enter navigation, unit
  tests and a Commands ribbon button; no new daemon calls or dependencies.
- GitHub Actions CI in `.github/workflows/ci.yml`: on pushes and pull requests it
  runs the six-step matrix with `--locked` on every Cargo command that supports
  it, `cargo fmt --all -- --check`, Clippy `-D warnings`, the `-j 2` release
  build and a clean-tree check. The exact pushed SHA's run replaces repeated
  manual six-step matrices for later changes; no Cargo dependencies, manifests or
  lockfile changes were added.
- Extended Help shortcut list and wired aliases (`Ctrl+O`, `Ctrl+,`, `Ctrl+1`,
  `Ctrl+2`, `Ctrl+F`, `Ctrl+Shift+Enter`, `Esc`). Design-only shortcuts for
  still-disabled controls are named in Help and tracked in `state.md`.

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
- Fix the GUI Properties dialog's `torrent_display` tuple destructure and clean
  the rustc warnings surfaced by `cargo test --workspace` (unused imports,
  unused fields and test-only helpers) without waiving `-D warnings`.
- Fix `E0499` borrow errors in the Actions and Properties windows by using a
  separate `close_clicked` flag instead of mutating the `Window::open` flag
  inside the `show` closure.
- Replace `std::io::Error::new(ErrorKind::Other, ...)` with
  `std::io::Error::other(...)` in the CLI IPC client to satisfy
  `clippy::io_other_error` under `-D warnings`.
- Clear the remaining `cargo clippy --workspace --all-targets --locked --
  -D warnings` findings surfaced by the second STEP 7 run: add `Default`
  implementations, box the two large `Decision` variants, use `checked_div`,
  `clamp`, range `contains`, `contains_key`, `next_back`, explicit
  `truncate(false)`, elided lifetimes and `OsStr` comparison, and replace the
  1.82-only `Option::is_none_or` with a `match`. Set
  `too-many-arguments-threshold = 12` in `clippy.toml` for the daemon's wide
  internal transfer/repository signatures; this is a project lint
  configuration, not an `#[allow]` or a `-D warnings` waiver, and the
  parameter-struct refactor is tracked in `state.md`.
- Replace two `partial_name.unwrap()` calls in `crates/core/tests/task_actions_ipc.rs`
  with nested `if let` bindings after a warning-only `cargo clippy
  --workspace --all-targets --locked` run confirmed they were the only
  remaining warnings at the previous SHA.
- Clarify Linux-only daemon support and the Linux GNU torrent target in
  `README.md`.
- Mark `docs/torrent-phase-five.md` as an append-only engineering log and add a
  finalization status section.
- Record PR #2 merged as `524a35b…` and PR #3 merged as `c9e8a8b…`, the green
  six-step GUI-only matrix at `8bd754e…` and the green docs-only matrix at
  `1c26bf9…` in `state.md`, `docs/finalization.md` and `README.md`; these were
  documentation-only changes with no code, dependency or lockfile edits. PR #3's
  body is the authoritative green record at `1c26bf9…`.

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
- The nine-step matrix (workspace tests, no-default-features tests, fault
  suite, public harness, split live suite, `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo build --release --locked -j 2`, clean tree) was green at `6cc0baf…`;
  `gh pr merge 1 --squash` then produced `99c845f…` on `master`.
- The GUI-only follow-up's six-step matrix (`cargo check -p downloader-gui
  --locked`, workspace tests, fmt, Clippy `-D warnings`, release build, clean
  tree) was green at `8bd754e…` with no warnings and no `-D warnings` waiver;
  `gh pr merge 2 --squash` then produced `524a35b…` on `master`. PR #2's body is
  the authoritative green record. No skipped release build, tag, Release,
  checksums or CI workflow existed for PR #1–#3.
- The docs-only follow-up's six-step user-side matrix was green at `1c26bf9…`
  before `gh pr merge 3 --squash` produced `c9e8a8b…` on `master`; PR #3's body
  is the authoritative green record. All three PRs predate CI.
- `.github/workflows/ci.yml` now runs the same six-step matrix on every push and
  pull request. The check, test, Clippy and release-build commands all keep
  `--locked`; Clippy keeps `-D warnings`; the release build keeps `-j 2`; no
  step is skipped. The GitHub Actions run at the exact pushed SHA is the
  verification record for later changes, so repeated manual six-step matrices
  are no longer required. Results are reported only from an actually observed
  GitHub Actions run.

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
- GUI appearance/behavior preferences are now persisted locally; editable
  categories and other in-memory GUI view state are not. Upload/seeding, RSS,
  labels, favorites, editable categories, magnets, sounds, native file picker
  and file mutations are disabled in the ported GUI and tracked in `state.md`.
- CI runs the deterministic six-step matrix only; real GUI, live-daemon and
  public-torrent-network checks remain manual/user-side. No assistant-side real
  Cargo/Clippy, GUI or peer-connection results are claimed.
- The earlier CachyOS single-file public run was a smoke test only and does not
  satisfy the multi-file acceptance criterion.
