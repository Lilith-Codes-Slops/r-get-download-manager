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

Build the daemon and GUI together with `cargo build --workspace --locked`,
then run `./target/debug/downloader-gui`. The GUI starts or connects to the
local daemon and polls its queue; restart any older running daemon after an
upgrade so it understands the detailed queue protocol.

The GUI is an `eframe`/`egui` port of the browser design mockup in
[`docs/gui-design/index.html`](docs/gui-design/index.html). It keeps the ribbon,
action bar, collapsible left rail, two-pane Tasks view, Downloads library,
tabbed Settings dialog and status bar. Only **core IPC** is wired: add URL
(HTTP/HTTPS or a local `.torrent` path), pause/resume/cancel/retry,
remove/delete, bulk pause/resume/remove-completed fan-out, the live detailed
queue and progress stream, split segment telemetry, live split-worker resizing,
a global speed limit, and the daemon-wide **Open containing folder**
post-download setting. See [`state.md`](state.md) for the exact wired/disabled
matrix and the long-term backlog. Controls whose daemon support is not ready are
visible but disabled with `// TODO(state.md):` comments in the source rather than
hidden.

Theme, compact mode, confirm-before-delete, in-app alerts, rail pinning, sort
order and the active Settings tab are GUI-only preferences persisted locally in
`gui-preferences.json` under `$XDG_CONFIG_HOME/rget/` or `~/.config/rget/` and
restored on startup. `Ctrl+K` opens a searchable command palette over the wired
actions; Help lists every wired shortcut and names the design-only shortcuts
that stay disabled. The Downloads view lists regular files from `~/Downloads`,
not a persistent history; opening files uses the system file association. Upload,
seeding, RSS, labels, favorites, editable categories, magnets, sounds, file
mutations and the remaining post-download automation are deferred. The GUI uses
the existing local IPC transport, not an HTTP/WebSocket service. Linux is the
only currently supported daemon platform; the torrent path targets Linux GNU.

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
[Phase 5 notes](docs/torrent-phase-five.md) for evidence and current gates.

### Manual live verification and IPC contract

See the [isolated manual verification procedure](docs/manual-live-verification.md)
for real-host, GUI, checkpoint-kill and v1 checks, and the
[finalization record](docs/finalization.md) for the completed PR #1 merge and
the current GUI follow-up verification list. The current live connection and
segment event shapes are recorded in the
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
