# Manual live verification (Linux)

These are **instructions, not completed results**. Use an authorized, stable
HTTP(S) test file of at least 1 GB with a trusted published SHA-256. You need
space for the file, sufficient bandwidth, and a host that supports bounded byte
ranges, a strong ETag and a known length. Do not deliberately overload a host
to elicit 429/503 responses. Record the URL, date, environment, observed results
and any errors; redact signed URLs or other secrets before sharing logs.

## Isolated setup

Run from the repository root. Close any previous lab GUI first. This uses a
separate home, state database and daemon socket; it does not alter your normal
queue. Keep this terminal open: later commands use its environment and PID.

```bash
cargo build --workspace --locked
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RGET_LAB="$(mktemp -d "${TMPDIR:-/tmp}/rget-lab.XXXXXX")"
mkdir -p "$RGET_LAB/run" "$RGET_LAB/state" "$RGET_LAB/Downloads"
# Keep the real desktop authentication paths when isolating HOME/runtime.
export XAUTHORITY="${XAUTHORITY:-$HOME/.Xauthority}"
if [ -n "${WAYLAND_DISPLAY:-}" ] && [ "${WAYLAND_DISPLAY#/}" = "$WAYLAND_DISPLAY" ] && [ -n "${XDG_RUNTIME_DIR:-}" ]; then
  export WAYLAND_DISPLAY="$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
fi
export HOME="$RGET_LAB" XDG_RUNTIME_DIR="$RGET_LAB/run" XDG_STATE_HOME="$RGET_LAB/state"
echo "Lab files: $RGET_LAB"
wait_for_daemon() {
  for _ in {1..150}; do
    if grep -q 'daemon listening' "$1"; then return 0; fi
    if ! kill -0 "$2" 2>/dev/null; then echo "Daemon exited; check $1"; return 1; fi
    sleep 0.1
  done
  echo "Daemon did not start; check $1"; return 1
}
./target/debug/downloader-core >"$RGET_LAB/daemon.log" 2>&1 &
RGET_PID=$!
wait_for_daemon "$RGET_LAB/daemon.log" "$RGET_PID" && ./target/debug/downloader-gui &
```

If the GUI cannot connect to your desktop under this isolated environment, stop
and use an environment that supports it; do not claim a GUI check was done.
GUI-triggered daemon autostart can complicate kill tests: close the lab GUI
before killing the daemon, and manually start the daemon before reopening it.

Before adding a URL, check its response headers. Replace the example with your
own URL and keep the quotes:

```bash
URL='https://your-authorized-host.example/large-file'
curl -sSIL -H 'Accept-Encoding: identity' "$URL"
curl -sSL -D - -o /dev/null --max-filesize 1024 \
  -H 'Accept-Encoding: identity' -H 'Range: bytes=0-0' "$URL"
```

The final full response should report at least 1 GB, `Accept-Ranges: bytes`,
and a strong ETag (quoted, without `W/`). The final probe response should be
`206` with `Content-Range: bytes 0-0/<total>`, length 1 and the same ETag.
The small `--max-filesize` limit prevents downloading a large body if a server
ignores the probe; curl may report an error in that case. These checks do not
replace the daemon's validation. If the host fails them, use another host and
record this as a fallback observation, not a split-engine verification.

## Large transfer and live controls

1. In the lab GUI, add the URL with 8 connections. Select the task and open
   **Network & splits**. Record requested maximum, active workers, segment
   progress and any fallback/rejection note. Active means workers owning ranges,
   not sockets, and may be below the requested maximum.
2. While the transfer is still running, set the requested maximum to 16, then
   to 2. Confirm the requested value changes and observe active counts. Increases
   need stealable work; reductions drain current segments, so neither outcome is
   necessarily instantaneous. Note if the resource finishes too quickly to
   observe the change; retry with a larger file or a controlled rate.
3. Let it finish. Compare the downloaded file's SHA-256 with the publisher's
   independently obtained digest using `sha256sum "$RGET_LAB/Downloads/<filename>"`
   (replace `<filename>` with the actual filename). Record match or mismatch.

## Exact split checkpoint-boundary kill

This requires a **test-only fault-injection build**. Never use it for normal or
production downloads. Use a fresh lab directory or a different test filename;
completed tasks and destination files are not overwritten.

1. Close the lab GUI. In the same terminal, stop the lab daemon with
   `kill "$RGET_PID"` and wait for it to exit with `wait "$RGET_PID"`.
2. Build the instrumented daemon:

   ```bash
   cargo build -p downloader-core --features checkpoint-test-hooks --locked
   rm -f "$RGET_LAB/checkpoint-marker"
   RGET_TEST_CHECKPOINT_HOOK=sidecar_committed \
   RGET_TEST_CHECKPOINT_MARKER="$RGET_LAB/checkpoint-marker" \
     ./target/debug/downloader-core >"$RGET_LAB/daemon-hook.log" 2>&1 &
   RGET_PID=$!
   wait_for_daemon "$RGET_LAB/daemon-hook.log" "$RGET_PID" && ./target/debug/downloader-gui &
   ```

3. Start an eligible split download with 8 connections. The hook pauses the
   checkpoint after sidecar commit once at least two segments have progress. In
   the terminal, check `ls -l "$RGET_LAB/checkpoint-marker"`. Wait until it
   exists; if it never appears, record the result and inspect the daemon log.
   It may help to set a speed limit before the transfer so it stays in progress.
4. Before restarting, preserve a read-only copy of the checkpoint for comparison
   with server logs. Then close the lab GUI. Verify the displayed process is the
   lab daemon, and kill **only the PID started above**:

   ```bash
   mkdir -p "$RGET_LAB/checkpoint-copy"
   cp "$RGET_LAB"/Downloads/.rget-*.part.meta "$RGET_LAB/checkpoint-copy/"
   ps -p "$RGET_PID" -o pid,args
   kill -9 "$RGET_PID"
   wait "$RGET_PID" 2>/dev/null || true
   ```

5. Restart without hook variables, then reopen the GUI:

   ```bash
   ./target/debug/downloader-core >"$RGET_LAB/daemon-restart.log" 2>&1 &
   RGET_PID=$!
   wait_for_daemon "$RGET_LAB/daemon-restart.log" "$RGET_PID" && ./target/debug/downloader-gui &
   ```

   Record recovery state and compare the final file with the trusted SHA-256.
   To establish that requests resume only from verified ranges, also compare the
   saved v2 sidecar segment prefixes with **server-side range request logs** or
   equivalent HTTP instrumentation. GUI progress and the final digest alone do
   not prove the exact resumed request offsets. Do not edit checkpoint files.
   A process kill is not a simulation of every power-loss interleaving.
6. Afterward close the GUI, stop the lab daemon, and rebuild without the
   test-only feature using `cargo build --workspace --locked` before normal use.

## Rejection and v1 checks

After rebuilding without the hook, start the lab daemon and GUI again using the
startup commands above. Use a fresh lab or different test filenames as needed.

- With an authorized server known to return 429 or 503 for excess range
  requests, record requested/active counts and the exact GUI rejection note;
  confirm whether the task continues and validate its final digest. A mock
  rejection test is useful but is not a real-host observation.
- Start a separate range-capable download with **1** connection. Wait for a v1
  sidecar to record a nonzero prefix. For example, this prints only sidecar
  version, sequence and downloaded prefix in the isolated lab:

  ```bash
  python3 -c 'import glob,json,os; [(lambda d: print(os.path.basename(p), d.get("version"), d.get("sequence"), d.get("downloaded")))(json.load(open(p))) for p in glob.glob(os.path.join(os.environ["HOME"], "Downloads", ".rget-*.part.meta"))]'
  ```

  Save a copy of that sidecar, close the lab GUI, kill the lab daemon PID,
  restart it and reopen the GUI. Compare the saved v1 prefix with server-side
  range logs and validate the final digest. A v1 checkpoint cannot currently be
  converted to split mode through SetConnections.
- Optional pooling measurement (local test server, not real-world proof):

  ```bash
  cargo test -p downloader-core --lib split::connection_tests::reports_connection_reuse --locked -- --nocapture
  ```

When finished, close the lab GUI and stop its daemon with `kill "$RGET_PID"` and
`wait "$RGET_PID"`. The lab directory remains for inspection; remove it only
after you have reviewed its contents and saved any results you need.
