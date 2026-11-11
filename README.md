# rget

High-performance Rust download manager for Linux.

> Torrent support targets Linux GNU. Windows daemon support is not implemented.

## Build

```bash
# Clean previous daemon socket if any (only if no production daemon is running)
pkill -f downloader-core || true
rm -f "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/rget.sock"

# Release build of everything
cargo build --release --locked

# Create a folder and copy the binaries
mkdir -p ~/rget
cp target/release/downloader-core ~/rget/
cp target/release/dl ~/rget/
cp target/release/downloader-gui ~/rget/ 2>/dev/null || true
cp target/release/downloader-nmhost ~/rget/ 2>/dev/null || true
```

## Run

Start the daemon (auto-spawned by clients):

```bash
./target/release/downloader-core
```

## CLI:

```bash
./target/release/dl add https://example.com/file.iso
```

## Arch:
```bash
# 1. Systemd user service
mkdir -p ~/.config/systemd/user
cat > ~/.config/systemd/user/rget.service << 'EOF'
[Unit]
Description=rget download daemon

[Service]
ExecStart=%h/rget/downloader-core
Restart=on-failure
WorkingDirectory=%h/rget

[Install]
WantedBy=default.target
EOF

systemctl --user daemon-reload
systemctl --user enable --now rget.service
systemctl --user status rget.service

# 2. Desktop entry
mkdir -p ~/.local/share/applications
cat > ~/.local/share/applications/rget.desktop << 'EOF'
[Desktop Entry]
Name=rget
Comment=Download manager
Exec=/bin/sh -c 'exec "$HOME/rget/downloader-gui"'
Icon=download
Terminal=false
Type=Application
Categories=Network;FileTransfer;
EOF

update-desktop-database ~/.local/share/applications 2>/dev/null || true

# 3. CLI on PATH
echo 'export PATH="$HOME/rget:$PATH"' >> ~/.bashrc
source ~/.bashrc

# 4. Verify
which dl
systemctl --user is-active rget.service
```
## Current limitations

The Linux daemon supports single-stream and experimental split HTTP/HTTPS
downloads. The CLI can request 1–32 connections using
`dl add <URL> --connections 2`; the GUI's Add download dialog selects 1–32
(default 8). The task view shows the requested ceiling and active workers
currently owning ranges, not TCP sockets. Split ownership transitions trigger
updates, and queue requests read the current map; displays can lag briefly in
transit. Unavailable telemetry from older daemons is shown as unknown. Split progress is delivered separately as throttled segment events;
when the event stream is unavailable the GUI falls back to queue samples. Live connection changes are available only for split tasks; existing
single-stream tasks cannot yet be converted to split mode. When a known-length
resource advertises byte ranges and a strong ETag, the daemon probes an exact
range and starts with segments of at least 1 MiB. Idle workers may steal an
unwritten suffix when it meets a response-latency-dependent floor, coupled to
the current sizing target. The floor can temporarily fall to 1 MiB after
congestion. The donor retains at least 1 MiB of unreserved work. Adaptive concurrency starts with up to two workers, increases on
healthy recent throughput up to the requested maximum, and reduces on a sharp
slowdown or an HTTP 429/503 response. It may run with one worker when a server
limits connections; repeated 429/503 responses at one worker eventually fail
rather than retry forever. Other invalid responses still fail closed. This
heuristic may mistake throttling or server stalls for congestion, and it does
not yet honor Retry-After or adapt segment sizes to worker speeds. Unsupported
range probes can fall back to a single stream. Split checkpoints use a v2 sidecar with a hash
for each segment and SQLite metadata; startup attempts to validate and resume
the recorded ranges. Split pause requests stop and join workers and checkpoint
before the task reports paused; paused split downloads can be resumed after a
restart. A split task's requested worker maximum can be changed while running
or paused. Decreases allow selected workers to finish their current segments;
increases may create workers when there is stealable work. Actual worker counts
can be lower than requested due to work availability or adaptive limits. Cancel
stops and joins split workers before cleaning up.
These changes require local build and integration testing before production use.
Queue entries and rate-limit settings are
stored in `$XDG_STATE_HOME/rget/queue.sqlite3` (or `~/.local/state/rget/queue.sqlite3`).
For eligible single-stream downloads, the daemon writes a synced `.part.meta`
checkpoint beside the partial file and records it in SQLite. On startup it
attempts to reconcile task-owned checkpoints, verify the partial-file hash, and
resume with a validated HTTP range. Paused tasks remain paused. Tasks without a
valid checkpoint are marked failed; a retry starts from the beginning. See
[checkpoint formats and recovery rules](docs/checkpoints.md). Orphaned
partial files are not adopted automatically. Pause and resume commands require
the daemon to be running; they apply to eligible single-stream and split downloads
from servers that advertise byte ranges, a known length and a strong ETag. Resume validates
the ETag, destination URL and exact Content-Range before appending. To pause or
resume, use `dl pause <URL>` and `dl resume <URL>`; wait until the queue shows
`paused` before resuming. Mirrors, extraction and scheduling are not yet
implemented. `dl rm <URL>` removes a task and cancels an active
transfer without deleting a completed file. `dl rm <URL> --delete-file` also
permanently removes the recorded completed file and any recorded partial/sidecar
files; missing files are ignored. Files are not moved to Trash. Files whose
ownership was not recorded (including older orphaned partials) are left alone.
`dl limit <kbps> [URL]` sets a global or per-task limit in decimal kilobits
per second; use `0` for unlimited. Limits persist across daemon restarts and
throttle download body processing, so network buffers can briefly exceed the requested rate. Existing files are never
replaced. Downloads are written to a temporary `.part` file and published only
after completion; a crash may leave a temporary file behind. If the destination
already exists, the daemon reports it before downloading rather than overwriting
it. If a file appears at that path during the transfer, the completed partial is
preserved at the path shown in the error for manual recovery. Browser integration offers an explicit context-menu handoff, but does
not cancel browser downloads automatically. Windows daemon support is not yet
implemented.

### GUI

The GUI is being rebuilt as a native `eframe`/`egui` prototype, using
[`docs/gui-design/index.html`](docs/gui-design/index.html) as a visual and
behavioral reference. It uses a **simulated** backend only: neither the sample
items nor actions control the real downloader daemon. The old daemon-connected
GUI sources remain in `crates/gui/src` but are not compiled by the prototype
entry point.

Icons are drawn as theme-colored native vector strokes in `crates/gui/src/icons.rs`
(at 16 or 18 logical pixels); `crates/gui/assets/icons/` is reserved for
future assets and contains no icon files.

The wgpu renderer prefers a compatible integrated GPU, then discrete or other
hardware, with software rendering only as a last resort. On Linux it prefers
Vulkan among adapters of the same type. The chosen adapter, device type and
backend are printed to stderr at startup; actual availability depends on the
driver and display surface. Set `RGET_GPU_DIAGNOSTICS=1` to list enumerated
adapters and eligibility checks when troubleshooting selection. `WGPU_BACKEND`
can restrict the backends wgpu considers. Driver enumeration can still load
other GPU libraries. The on-demand
egui font atlas is limited to a 4096-pixel side (or the device limit if smaller),
while system-font fallback remains enabled. GPU drivers and display settings
influence memory use; no specific RSS or responsiveness is guaranteed.

The prototype creates `data/state.json` relative to its working directory on
first launch, with sample tasks, a seed and a library item. Changes are saved
there, and an existing empty state is not reseeded. Invalid state files are not
overwritten. The file is ignored by Git. The Add download dialog accepts up to five
nonempty link entries and creates simulated tasks; it does not validate or fetch them.
Properties for one selected
task, seed, or library item are available from the row context menu or More menu. The prototype's native
torrent chooser records a simulated task, not a real download; on Linux it requires `zenity` or
`kdialog`. The rail and its side panels use the same simulated state; its pin is
session-only. The Tasks view has independently searchable Active and Transfers
panes, with both running and stopped seeds shown in Transfers. The panes stack on
narrow windows. Task rows include status-colored
progress, eight segments, and pause/resume/retry controls; seeds can be stopped
and restarted. Rows support multi-selection and compact spacing. The Downloads library supports
search and editable, persisted categories. Settings (action bar or Ctrl/Cmd+,)
are saved immediately, including theme, compact mode, alerts, delete confirmation,
concurrency, speed limit and seed ratio. The speed limit is shared across the
Downloads and Network sections. Sound switches are disabled: this prototype never
plays audio. Row context menus and ribbon Help/Commands dropdowns operate on the fake
state. Keyboard shortcuts include Ctrl/Cmd+N or O to add, Ctrl/Cmd+M (or
Ctrl/Cmd+Shift+M) for magnets, Ctrl/Cmd+F for search, Ctrl/Cmd+P/R for pause
and resume all, Ctrl/Cmd+1/2 for views, Ctrl/Cmd+B to pin the rail, and
Space/Delete/Escape for selection and dismissal. Empty files in `sounds/` are
placeholders and are never played. Copy and Cut write actual stored file paths or source URLs to the system
clipboard (one per line); Cut never moves files in this preview. Delete from disk
is also simulated and never deletes real files. Clipboard errors are reported;
Linux Wayland requires compositor data-control support. The simulation advances
every 500 ms with the reference's speed, concurrency, failure and seeding-ratio
rules. It continues while the window is minimized or unfocused, and atomically
saves changed state to `data/state.json`. Completed tasks move to the simulated
library; no real network transfer or file creation occurs.

Build with `cargo build -p downloader-gui --locked`, then run
`./target/debug/downloader-gui` from the desired working directory.

### Verifying a completed download

Use `dl verify <task-URL> sha256 <expected-hex>` or replace `sha256` with
`blake3`. Verification is explicit and applies only to completed tasks; it is
not yet an automatic post-download action. A mismatch returns an error.

### Post-download actions

Settings includes one global post-download action: **Open containing folder
after a download completes**. When enabled, the daemon opens the completed
file's parent directory, or the published torrent root directory, in the
desktop file manager with `xdg-open`. The setting is stored in the daemon's
SQLite state, so it applies even when the GUI is closed. It is skipped on
headless daemons without a graphical session environment and is not applied
retroactively to tasks completed before the daemon started.

The tabbed Settings dialog exposes this global toggle under Post-download
actions, and the redesigned Post-download actions tab shows its current value
with a link to Settings. Other placeholder actions (Close rGet, Verify checksum,
Extract archive, Run antivirus scan, System power action) remain unimplemented
and are tracked in [`state.md`](state.md).

### V1 torrents

Local BitTorrent v1 `.torrent` files are supported on Linux GNU. The daemon
preflights ordinary v1 multi-file metadata, stages to a private tree, verifies
pieces after storage quiescence, publishes the directory with no-replace
semantics, and exposes root/file-count identity to the GUI. Magnets for
multi-file torrents, v2/hybrid/private torrents, padding/symlink/special files,
webseed retrieval, upload and seeding are not supported. Webseed declarations
are ignored, so reachable BitTorrent peers are required.

An opt-in ignored integration test can exercise a public v1 multi-file torrent
with `RGET_PUBLIC_TORRENT`; a separate single-file smoke test uses
`RGET_PUBLIC_SINGLE_TORRENT`. The single-file smoke test does not satisfy the
multi-file acceptance criterion. The user reported that the public v1
multi-file test passed on their PC; the verbatim final log was not attached to
this branch. See [finalization status](docs/finalization.md) and
[Phase 5 notes](docs/torrent-phase-five.md) for the completed merges, green
verification records and historical evidence.

### Continuous integration

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs the six-step
verification matrix on every push and pull request, covering `master`, `main`
and any other base branch without changing the PR base. The check, test, Clippy
and release-build commands all use `--locked`; `cargo fmt --all -- --check`,
Clippy with `-D warnings` and the `-j 2` release build are all included. The
GitHub Actions run at the exact pushed SHA is the verification record for later
changes, so the repeated manual six-step matrices in
[`docs/finalization.md`](docs/finalization.md) are historical evidence rather
than a recurring gate. CI does not run real GUI, live-daemon or
public-torrent-network checks; those remain manual/user-side.

### Manual live verification and IPC contract

See the [isolated manual verification procedure](docs/manual-live-verification.md)
for real-host, GUI, checkpoint-kill and v1 checks, and the
[finalization record](docs/finalization.md) for the completed PR #1, PR #2 and
PR #3 merges and their green user-side verification records. The historical
six-step command list is in [`state.md`](state.md) and
[`docs/finalization.md`](docs/finalization.md).
The current live connection and segment event shapes are recorded in the
[IPC contract](docs/ipc-segment-contract.md).

### Split checkpoint hardening tests (Linux)

Split workers reserve each positioned-write range under the map lock, release the
lock while writing, and commit progress only after the write completes. A steal
can take only bytes beyond any reservation. Checkpoint snapshots include only
committed prefixes; file sync, sidecar replacement and SQLite commit happen
without holding the map lock. Uncheckpointed bytes may be overwritten on resume.

The normal test suite includes concurrent reservations, a checkpoint taken
while a write is in flight, a shared task-rate test, a split task-rate test, and
a local HTTP/1.1 keep-alive measurement. To print the number of range requests
and accepted TCP connections (including the probe) for that measurement:

```sh
cargo test -p downloader-core --lib split::connection_tests::reports_connection_reuse --locked -- --nocapture
```

In one local run, the test observed 5 range requests over 2 TCP connections,
including the probe. This is workload-specific: response cancellation, steals,
server policies and HTTP version can all change connection reuse. A shared
`reqwest::Client` permits pooling but does not guarantee reuse.

Real daemon-kill tests and the controlled in-flight-write test require a
**test-only fault-injection feature**. Never enable it for a production daemon:

```sh
cargo test -p downloader-core --test split_live_ipc --features checkpoint-test-hooks --locked
```

These tests stop the daemon after data sync, sidecar rename (before directory sync), sidecar commit, and SQLite commit,
then restart and verify the complete output. This does not prove every possible
power-loss, runtime shutdown, or filesystem failure scenario. In particular, a
daemon-wide graceful-shutdown barrier for blocking writes remains outstanding.

## License

Dual-licensed under `MIT OR Apache-2.0`; see [LICENSE](LICENSE),
[LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
