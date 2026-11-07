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

The Linux daemon supports single-stream and experimental fixed-split HTTP/HTTPS
downloads. The CLI can request 1–32 connections using
`dl add <URL> --connections 2`; the GUI still requests one. When a known-length
resource advertises byte ranges and a strong ETag, the daemon probes an exact
range and starts with segments of at least 1 MiB; idle workers may steal half
of a sufficiently large unfinished range. Each resulting remaining half is at
least 1 MiB. Unsupported range responses fall back to a single stream. Split checkpoints use a v2 sidecar with a hash
for each segment and SQLite metadata; startup attempts to validate and resume
the recorded ranges. Split pause and mid-download connection changes are not yet
supported. Cancel stops and joins the split workers before cleaning up.
These changes require local build and integration testing before production use.
Queue entries and rate-limit settings are
stored in `$XDG_STATE_HOME/rget/queue.sqlite3` (or `~/.local/state/rget/queue.sqlite3`).
For eligible single-stream downloads, the daemon writes a synced `.part.meta`
checkpoint beside the partial file and records it in SQLite. On startup it
attempts to reconcile task-owned checkpoints, verify the partial-file hash, and
resume with a validated HTTP range. Paused tasks remain paused. Tasks without a
valid checkpoint are marked failed; a retry starts from the beginning. Orphaned
partial files are not adopted automatically. Pause and resume commands require
the daemon to be running; for now they apply only to single-stream downloads
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
upgrade so it understands the detailed queue protocol. Add HTTP/HTTPS URLs
from the toolbar, select tasks to inspect progress, or browse files in the
Downloads view. The displayed transfer rate is the average for the current
transfer; upload/seeding is not supported. Theme overrides are session-only.

The Downloads view lists regular files from `~/Downloads`, not a persistent
history. Opening files uses the system file association. The daemon's queue is
saved in SQLite. Eligible incomplete downloads can recover after restart as
described above. The GUI exposes pause and resume for eligible tasks; Cancel
stops an active transfer and keeps its history, while Delete asks for confirmation
and permanently removes the task's recorded files and history. Failed or canceled
tasks can be retried. GUI rate limiting and other disabled controls remain
unavailable. User-directed split resizing, adaptive concurrency, torrent/Metalink
support, file mutations and post-download automation require engine or platform work. The GUI uses the
existing local IPC transport, not an HTTP/WebSocket service. Linux is the only
currently supported daemon platform.

### Verifying a completed download

Use `dl verify <task-URL> sha256 <expected-hex>` or replace `sha256` with
`blake3`. Verification is explicit and applies only to completed tasks; it is
not yet an automatic post-download action. A mismatch returns an error.
