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
in-memory task list on Linux. Tasks are not recovered after a restart. Pause,
resume, removal, rate limits, mirrors, extraction and scheduling are not yet
implemented; requests for them return an error. Existing files are never
replaced. Downloads are written to a temporary `.part` file and published only
after completion; a crash may leave a temporary file behind. Browser integration offers an explicit context-menu handoff, but does
not cancel browser downloads automatically. Windows daemon support is not yet
implemented.
