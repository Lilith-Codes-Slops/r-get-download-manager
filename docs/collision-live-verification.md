# Manual collision and retry verification (Linux)

These are instructions, **not test results**. The fixture serves generated data
only on `127.0.0.1`; it intentionally sends an invalid ETag in bad mode. Use it
only for this isolated lab, not with real downloads. Reserve at least 200 MiB
of disk space. These checks cover single-stream recovery; they do not replace
split crash-boundary tests or the large-file checklist in
[manual-live-verification.md](manual-live-verification.md).

## 1. Build and start an isolated lab

Open a terminal in the repository root. Copy this block into that terminal;
keep it open because later commands use its variables. Close any previous lab
GUI first. If the fixed port is in use, stop the other service or choose a
free port and replace **all** instances of `18765` below consistently. First,
build the project and continue only if the build succeeds:

```bash
cargo build --workspace --locked
```

Then start the lab in the same terminal:

```bash
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RGET_LAB="$(mktemp -d "${TMPDIR:-/tmp}/rget-collision.XXXXXX")"
mkdir -p "$RGET_LAB/run" "$RGET_LAB/state" "$RGET_LAB/Downloads"
export XAUTHORITY="${XAUTHORITY:-$HOME/.Xauthority}"
if [ -n "${WAYLAND_DISPLAY:-}" ] && [ "${WAYLAND_DISPLAY#/}" = "$WAYLAND_DISPLAY" ] && [ -n "${XDG_RUNTIME_DIR:-}" ]; then
  export WAYLAND_DISPLAY="$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
fi
export HOME="$RGET_LAB" XDG_RUNTIME_DIR="$RGET_LAB/run" XDG_STATE_HOME="$RGET_LAB/state"
echo "Lab directory: $RGET_LAB"
python3 docs/collision-lab-server.py --port 18765 >"$RGET_LAB/server.log" 2>&1 &
RGET_SERVER_PID=$!
./target/debug/downloader-core >"$RGET_LAB/daemon.log" 2>&1 &
RGET_PID=$!
for _ in $(seq 1 150); do
  if grep -q 'Collision lab listening' "$RGET_LAB/server.log" && grep -q 'daemon listening' "$RGET_LAB/daemon.log"; then break; fi
  if ! kill -0 "$RGET_SERVER_PID" 2>/dev/null || ! kill -0 "$RGET_PID" 2>/dev/null; then break; fi
  sleep 0.1
done
if grep -q 'Collision lab listening' "$RGET_LAB/server.log" && grep -q 'daemon listening' "$RGET_LAB/daemon.log"; then
  ./target/debug/downloader-gui &
else
  echo "Startup failed; inspect the two logs in $RGET_LAB before continuing."
fi
```

The daemon uses the isolated `Downloads`, queue database and socket. If the GUI
cannot connect to your desktop, stop here; don't count the GUI checks as passed.
The fixture sends 32 MiB per file at a deliberately slow rate. The two commands
below switch whether **new range responses** have a correct or intentionally
wrong validator. Do not switch modes while an unrelated lab transfer is active.

Bad mode (use only when instructed):

```bash
curl -fsS http://127.0.0.1:18765/__mode/bad
```

Good mode (restore before retrying):

```bash
curl -fsS http://127.0.0.1:18765/__mode/good
```

## 2. Failed same-URL retry resumes verified bytes

1. In the GUI, add `http://127.0.0.1:18765/resume` with **1 connection**.
   Wait for some progress, select it and click **Pause**. Wait until it says
   `paused`; record the downloaded byte count.
2. In the terminal run the **bad** command above. In the GUI click **Resume**.
   Wait for the task to fail due to the changed validator. It must not publish
   a completed `resume.bin` at this point.
3. Run the **good** command. In the GUI click **Retry** (or add the same URL
   again). Observe that the same URL task resumes; wait for completion. Inspect
   the fixture log:

   ```bash
   grep '/resume' "$RGET_LAB/server.log"
   ```

   Record a successful range request with a nonzero start and `If-Range` set to
   `"rget-lab-v1"`. Check `resume.bin` exists and `resume (1).bin` does not.
   A failed attempt must not be counted as a successful resume.

## 3. Corrupt checkpoint retry starts a renamed fresh download

1. Add `http://127.0.0.1:18765/corrupt` with **1 connection**. Pause after
   some progress and wait for `paused`. Switch to **bad** mode, resume, and
   wait until it fails. Switch back to **good** mode.
2. In the same terminal, run this block. It locates the single task-matched
   version-1 sidecar inside the isolated lab, saves a backup in the lab, and
   deliberately corrupts that sidecar. If it reports anything other than one
   matching sidecar, stop and inspect the lab instead of modifying other files.

   ```bash
   python3 - <<'PY'
   import json, os
   from pathlib import Path
   root = Path(os.environ['RGET_LAB'])
   url = 'http://127.0.0.1:18765/corrupt'
   matches = []
   for path in (root / 'Downloads').glob('.rget-*.part.meta'):
       if path.is_symlink() or not path.is_file():
           continue
       try:
           record = json.loads(path.read_bytes())
       except (ValueError, OSError):
           continue
       if record.get('version') == 1 and record.get('task_url') == url:
           matches.append(path)
   if len(matches) != 1:
       raise SystemExit(f'Expected one matching sidecar, found {len(matches)}; nothing changed.')
   sidecar = matches[0]
   backup = root / 'corrupt-sidecar-backup.json'
   backup.write_bytes(sidecar.read_bytes())
   sidecar.write_text('intentionally invalid checkpoint')
   print(f'Corrupted {sidecar}; backup: {backup}')
   PY
   ```

3. Click **Retry** for that task. It should begin a fresh download named
   `corrupt (1).bin`, with a **Renamed** indicator. Wait for completion. Check
   `corrupt.bin` was not published and the old partial remains in the lab.
   Record the results and any error; do not remove the old partial to make the
   check pass.

## 4. Concurrent and late name collisions

1. Add both `http://127.0.0.1:18765/collision?case=A` and
   `http://127.0.0.1:18765/collision?case=B` with **1 connection** each while
   they overlap. Both responses suggest `collision.bin`. Let both finish.
   Verify the directory contains `collision.bin` and `collision (1).bin`, and
   the renamed task shows **Renamed**. Both should contain the fixture data.
2. Add `http://127.0.0.1:18765/collision?case=C`. While it is still downloading,
   wait until its displayed destination is `collision (2).bin`. Then run:

   ```bash
   (set -o noclobber; printf 'late-collision-sentinel\n' > "$RGET_LAB/Downloads/collision (2).bin") || echo "Destination already exists; late-race check is inconclusive."
   ```

   If the command reports that the destination already exists, stop this
   check: the late-race result is inconclusive. Otherwise, let the task finish.
   It should publish `collision (3).bin`, still show the
   badge, and leave the sentinel untouched. There should be no
   `collision (2) (1).bin`. If the transfer finishes before you create the
   sentinel, this late-race check is inconclusive; start a fresh lab and retry.

## 5. Compare data and stop the lab

For the five completed fixture downloads, compare every printed hash with the
expected hash. The sentinel is intentionally different and should not be
included.

```bash
python3 - <<'PY'
import hashlib
print('Expected:', hashlib.sha256(bytes(range(256)) * (32 * 1024 * 1024 // 256)).hexdigest())
PY
sha256sum "$RGET_LAB/Downloads/resume.bin" "$RGET_LAB/Downloads/corrupt (1).bin" \
  "$RGET_LAB/Downloads/collision.bin" "$RGET_LAB/Downloads/collision (1).bin" \
  "$RGET_LAB/Downloads/collision (3).bin"
cat "$RGET_LAB/Downloads/collision (2).bin"
```

Record pass/fail/inconclusive, filenames, hashes and relevant log lines. Close
the lab GUI **before** stopping its daemon, then run:

```bash
kill "$RGET_PID" "$RGET_SERVER_PID"
echo "Lab retained for inspection: $RGET_LAB"
```

This does not remove the lab directory or alter your normal queue. Closing and
reopening your terminal restores your usual environment. Do not delete lab
artifacts until you no longer need them for investigation.
