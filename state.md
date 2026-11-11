# rGet state / long-term roadmap

This is the living backlog for the GUI port. It records what is wired to the
daemon today, what is intentionally visible-but-disabled, and what we work on
next. Disabled controls in `crates/gui/src/app_state/views.rs` carry
`// Work on it later; TODO(state.md): ...` comments so the reason is visible at
the call site.

## Work on it later (long-term project)

The phrase "Work on it later" is intentional: this is a long-term project, and
the rows below are the next work items rather than merge blockers. Source-level
`#[allow(dead_code)]` annotations on CLI/native-host stubs and `#[cfg(test)]`
gates on reserved core helpers document that intent without weakening
`cargo clippy --workspace --all-targets --locked -- -D warnings`.

## Branch / PR

- Active branch: `arena/1c81e486-r-get-download-manager` (post-merge GUI-only slice).
- PR #1: **MERGED** with `gh pr merge 1 --squash`; head
  `6cc0baf7766142d4b3ab7daff0672736650c3129`, merge commit
  `99c845fd3f496008b2a585ae7592671f332eba68` on `master`.
- The nine-step user-side pre-merge matrix was green at `6cc0baf…`. No tag,
  GitHub Release, checksums or CI workflow were created.
- Design source: [`docs/gui-design/index.html`](docs/gui-design/index.html) — the
  single-file browser mockup the Rust `egui` GUI is ported from.
- Shipped GUI: `crates/gui` (`eframe`/`egui`), using the existing local
  `ClientRequest` / `DaemonEvent` IPC transport. There is no HTTP/WebSocket
  service.

## Wired now — core IPC only

| UI control | Implementation |
| --- | --- |
| `+ URL` / Add download | `ClientRequest::AddTask` for HTTP/HTTPS URLs and local ordinary v1 `.torrent` paths. `connections` is 1–32 per-task workers. |
| Pause / Resume / Cancel / Retry | `ClientRequest::Pause`, `Resume`, `Cancel`; Retry re-adds the source with `AddTask`. |
| Remove from list | `ClientRequest::RemoveTask { delete_file: false }` |
| Delete from disk | `ClientRequest::RemoveTask { delete_file: true }`, gated by Confirm before delete. |
| Pause All / Resume All / Remove Completed | Client-side fan-out of the single-task requests above; there is no daemon bulk API. |
| Live queue + progress | `GetDetailedQueue` polling and the `SubscribeProgress` stream (existing `IpcBridgeWorker` / `progress_stream.rs`). |
| Split segment telemetry | `SubscribeSegments` / `SubscribeSegmentPolicies` (existing `segments.rs` / `policies.rs`). |
| Live split-worker resize | `ClientRequest::SetConnections` for active/paused split tasks only. |
| Global speed limit | `ClientRequest::SetLimit { task_id: None, kbps }` in Settings → Downloads. `0` = unlimited; units are decimal kilobits/s. |
| Global post-download action | `GetPostDownloadSettings` / `SetPostDownloadSettings` in Settings. Default is **off**. The daemon opens the containing folder on completion, GUI-closed included. |
| Open Folder (GUI) | `xdg-open` / `explorer` / `open` on the task's `final_path` parent (or the root itself for a multi-file torrent), else `~/Downloads`. This is separate from the daemon-side setting. |
| Copy Link / Copy path | egui clipboard. |
| Properties | Read-only details window from `DownloadInfo`. |

## Work on it later — disabled until the daemon is fully prepared

Every row below has a matching `Work on it later; TODO(state.md)` comment next
to a disabled button, checkbox or placeholder in `views.rs`.

| Control | Why it is disabled | Next step |
| --- | --- | --- |
| `+ Magnet` | `AddTask` rejects non-HTTP(S) URLs; magnet ingest (especially multi-file) is not implemented. | Add a v1 magnet metadata path and preflight before calling `AddTask`. |
| `Add file` (native picker) | No file dialog is wired. Users can paste a local `.torrent` path into `+ URL` today. | Add a native file picker and pass the selected path to `AddTask`. |
| Start / Stop seeding, seed-until-ratio, upload speed | Seeding/upload and `TorrentMetrics` upload telemetry are out of scope; `Session::stop()` does not join engine tasks. | Implement upload scheduling, seed state and ratio enforcement. |
| RSS Feeds, Labels, Favorites | No daemon storage, fetcher or IPC surface. | Add persistence + requests before exposing the panels. |
| Recent (separate history) | The GUI uses the live daemon queue; there is no durable recent list. | Add a history table or derive from persisted queue. |
| Category `+ New` / Edit / Delete | `AddTask` rejects `category`; categories are currently in-memory GUI filters only. | Add a category store and accept `category` in `AddTask`. |
| Max concurrent downloads | No daemon API for a global queue cap; `connections` is per-task workers. | Add a scheduler cap and IPC set/get. |
| Sound on completion / failure | No GUI audio backend. | Add an audio backend and failure/completion hooks. |
| Verify checksum (post-download action) | `VerifyChecksum` exists but is CLI-only, not a completion action. | Add a `PostDownloadSettings` field and daemon hook. |
| Extract archive / Run antivirus scan / System power action / Close rGet | `crates/core/src/post_process/*` and `power/*` are stubs with no daemon call sites. | Implement each action behind the global settings struct. |
| Refresh URL / Add mirror / media quality / torrent-event UI | IPC variants exist but are not surfaced in the ported GUI yet. | Add dedicated detail tabs or dialogs. |
| Rename / move / delete in Downloads view | No daemon file-mutation API; the view lists `~/Downloads` directly. | Add a safe file-mutation request with ownership checks. |
| Manual daemon toggle | `IpcBridgeWorker` auto-spawns and reconnects; toggling would be fake. | Add an explicit connect/disconnect command if wanted. |

## GUI-only, wired (no daemon change needed)

- Ribbon, action bar, collapsible/pinned left rail, two-pane Tasks view,
  Downloads library, Settings tabs and status bar.
- Local preference persistence: theme, compact mode, confirm-before-delete,
  in-app alerts, rail pinning, sort field/direction and the active Settings tab.
  Versioned JSON lives at `gui-preferences.json` under `$XDG_CONFIG_HOME/rget/`
  or `~/.config/rget/`; missing, unreadable or malformed values fall back to
  defaults and are rewritten on the next change.
- `Ctrl+K` command palette over existing wired actions only, with
  case-insensitive label/shortcut/keyword search; the Help window lists every
  wired shortcut and names the design-only shortcuts that stay disabled.
- Active / Transfers search filters, Downloads search, category chips,
  name/size/date/type sorting, row selection.
- Keyboard shortcuts: `Ctrl+N`/`Ctrl+O` add, `Ctrl+S`/`Ctrl+,` settings,
  `Ctrl+P` pause all, `Ctrl+R` resume all, `Ctrl+B` pin/unpin rail,
  `Ctrl+K` command palette, `Ctrl+1` Tasks, `Ctrl+2` Downloads, `Ctrl+F`
  focus filter, `Ctrl+Shift+Enter` open Downloads folder, `Esc` close dialog.
- Downloads view file listing, Open file/folder, Copy path.

## Deferred / long-term

- Persist editable categories and other in-memory GUI view state; the
  daemon-side global speed limit is already stored by the daemon. Appearance and
  behavior preference persistence is done.
- Fast resume, magnets, v2/hybrid/private torrents, webseeds, file priorities,
  upload/seeding, global upload cap.
- Mirrors, extraction, scheduling, Metalink, Trash, automatic checksums,
  a daemon-wide graceful-shutdown barrier.
- Packaging smoke tests, browser-extension handoff lab, real 429/503 split
  observation, ≥1 GB split GUI run, checkpoint-boundary SIGKILL/restart.
- Refactor the daemon's wide internal transfer and repository function
  signatures into parameter structs, then remove
  `too-many-arguments-threshold = 12` from `clippy.toml` and restore the
  default `7` threshold under `-D warnings`.
- Windows daemon support.

## Verification status

- PR #1 was merged at `99c845f…` after the user-side nine-step matrix was green
  at `6cc0baf…` (workspace tests, no-default-features tests, fault suite,
  public harness, split live suite, `cargo fmt --all -- --check`, Clippy with
  `-D warnings`, `cargo build --release --locked -j 2`, and a clean tree). No
  `-D warnings` waiver, skipped release build, tag, Release or checksums.
- The current dependency-free GUI-only slice is on the active branch. Its own
  six-step user-side verification is pending at the new SHA:
  `cargo check -p downloader-gui --locked`, `cargo test --workspace --locked`,
  `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked
  -- -D warnings`, `cargo build --release --locked -j 2`, and a clean tree.
  The old matrix steps 2–5 remain optional full-parity checks.
- `state.md` is the long-term backlog, not a merge gate. Native file picker,
  sound on completion and manual daemon toggle stay disabled with their
  `TODO(state.md)` comments.
