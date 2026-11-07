# rget

High-performance Rust download manager for Linux and Windows.

## Build

```bash
# Clean previous daemon socket if any
pkill -f downloader-core || true
rm -f /run/user/1000/rget.sock

# Release build of everything
cargo build --release

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
ExecStart=/home/scarlet/rget/downloader-core
Restart=on-failure
WorkingDirectory=/home/scarlet/rget

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
Exec=/home/scarlet/rget/downloader-gui
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

The daemon currently supports single-connection HTTP/HTTPS downloads and an
SQLite-backed task list on Linux. Queue entries and rate-limit settings are
stored in `$XDG_STATE_HOME/rget/queue.sqlite3` (or `~/.local/state/rget/queue.sqlite3`).
For eligible single-stream downloads, the daemon writes a synced `.part.meta`
checkpoint beside the partial file and records it in SQLite. On startup it
attempts to reconcile task-owned checkpoints, verify the partial-file hash, and
resume with a validated HTTP range. Paused tasks remain paused. Tasks without a
valid checkpoint are marked failed; a retry starts from the beginning. Orphaned
partial files are not adopted automatically. Pause
and resume are available only while the daemon is running and only for servers
that advertise byte ranges, a known length and a strong ETag. Resume validates
the ETag, destination URL and exact Content-Range before appending. To pause or
resume, use `dl pause <URL>` and `dl resume <URL>`; wait until the queue shows
`paused` before resuming. Mirrors, extraction and scheduling are not yet
implemented. `dl rm <URL>` removes a task and cancels an active
transfer without deleting a completed file (`--delete-file` is not supported).
`dl limit <kbps> [URL]` sets a global or per-task limit in decimal kilobits
per second; use `0` for unlimited. Limits persist across daemon restarts and
throttle download body processing, so network buffers can briefly exceed the requested rate. Existing files are never
replaced. Downloads are written to a temporary `.part` file and published only
after completion; a crash may leave a temporary file behind. Browser integration offers an explicit context-menu handoff, but does
not cancel browser downloads automatically. Windows daemon support is not yet
implemented.

### GUI

Build the daemon and GUI together with `cargo build --workspace --locked`,
then run `./target/debug/downloader-gui`. The GUI starts or connects to the
local daemon and polls its queue; restart any older running daemon after an
upgrade so it understands the detailed queue protocol. Add HTTP/HTTPS URLs
from the toolbar, select tasks to inspect progress, or browse files in the
Downloads view. The displayed transfer rate is the average for the current
transfer; upload/seeding is not supported. Theme overrides are session-only.

The Downloads view lists regular files from `~/Downloads`, not a persistent
history. Opening files uses the system file association. The daemon's queue is
saved in SQLite, but incomplete downloads do not yet resume after restart. The
disabled controls are deliberately
unavailable: the GUI does not yet expose task removal or rate limiting;
pause/resume are currently CLI-only. Multiple connections, torrent/Metalink
support, file mutations and post-download automation require engine or platform work. The GUI uses the
existing local IPC transport, not an HTTP/WebSocket service. Linux is the only
currently supported daemon platform.

### Verifying a completed download

Use `dl verify <task-URL> sha256 <expected-hex>` or replace `sha256` with
`blake3`. Verification is explicit and applies only to completed tasks; it is
not yet an automatic post-download action. A mismatch returns an error.
