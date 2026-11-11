# Finalization status and merge gate

This document records the decisions made for the finalization flowchart and the
remaining actions for PR #1. It is deliberately conservative: a public-network
pass is reported by the user, but a verbatim final log was not attached to this
branch.

## Recorded decisions

| Node | Decision |
| --- | --- |
| Public multi-file acceptance | User-reported pass; verbatim final log not attached. Do not quote `test result: ok` as an observed line. |
| Earlier single-file smoke run | Not multi-file acceptance. Close it out and clean its isolated files on the user's PC. |
| Final matrix status | Cargo suites and public torrent run passed on the user's PC at an earlier SHA. Formatting/Clippy fixes, the post-download action feature and the GUI port are now applied; the complete matrix, including the release build, must be rerun at the final SHA. |
| Scope | Merge PR #1 only. No `v0.1.0` tag or GitHub Release in this flow. |
| Merge strategy | `--squash`, after all gates are green. |
| Current action | Push the consolidated Clippy cleanup commit, update PR metadata with its SHA, then wait for final matrix confirmation. **Do not merge yet.** |

The docs/hygiene commit, the subsequent formatting/Clippy fix commit, the
post-download action feature commit and the GUI port commit each change the
release SHA. The entire deterministic matrix must be rerun at the final SHA.
Earlier results at `251cb71`, `70c56e5`, `07fd541`, `65f5f1e` and the
post-download feature commit are useful history but are not sufficient to
unblock the merge.

## 1. Save evidence if it becomes available

The fallback wording is already used in `README.md`, `CHANGELOG.md` and
`docs/torrent-phase-five.md`:

> user-reported pass; verbatim final log not attached.

If the exact log is later available, add a redacted section here containing:

- exact commit SHA;
- exact public multi-file command;
- torrent URL, local path and info-hash;
- preflight/tracker output;
- final `completed: ... all v1 pieces verified` line;
- final `test result: ok` line;
- date, OS and toolchain versions;
- whether the isolated home was removed.

Redact signed URLs, credentials and private paths before pasting logs into the
PR. The candidate supplied for this run was the Internet Archive Alice in
Wonderland LibriVox `_archive.torrent`, info-hash
`603f7afe0174288bfee17e41399d066952d5251d`. Tracker/peer availability can change.

## 2. Clean the earlier single-file run on your PC

Review before deleting anything; save logs first. Do not kill a production
daemon by accident.

```sh
pgrep -af downloader-core || true
find /tmp -maxdepth 1 -type d \
  \( -name 'rget-public-single-*' -o -name 'rget-public-torrent-*' -o -name 'rget-lab.*' \) \
  -print
```

After reviewing the paths and stopping only lab daemon PIDs you started:

```sh
find /tmp -maxdepth 1 -type d \
  \( -name 'rget-public-single-*' -o -name 'rget-public-torrent-*' -o -name 'rget-lab.*' \) \
  -exec rm -rf -- {} +

rm -f "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/rget.sock"
unset RGET_PUBLIC_TORRENT RGET_PUBLIC_SINGLE_TORRENT RGET_PUBLIC_TIMEOUT_SECS
unset RGET_TORRENT_TEST_DIR RGET_TORRENT_TEST_POINT
unset RGET_TEST_CHECKPOINT_HOOK RGET_TEST_CHECKPOINT_MARKER RGET_TEST_WRITE_BLOCK
```

The CachyOS single-file run does not satisfy multi-file acceptance. Leaving it
out of scope is acceptable; do not describe its partial output as a pass.

## 3. Final matrix at the exact final SHA

On the user's PC, from the repository root:

```sh
git pull --ff-only origin arena/4f707bcb-r-get-download-manager
git status --short
git rev-parse HEAD

cargo test --workspace --locked
cargo test -p downloader-core --no-default-features --locked
cargo test -p downloader-core --features torrent-test-hooks \
  --test torrent_fault_ipc --locked -- --test-threads=1
cargo test -p downloader-core --test torrent_public_ipc --locked
cargo test -p downloader-core --test split_live_ipc \
  --features checkpoint-test-hooks --locked

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --locked
```

If the release build is memory-heavy, rerun it with `-j 2` (or
`CARGO_BUILD_JOBS=2`); this workspace links `wgpu`, `egui` and `librqbit`. If
`df` is aliased to `duf` on your shell, use `/bin/df -h "$HOME"` to check disk.

`git status --short` must be empty. If any command fails, fix it and rerun the
matrix. If Clippy warnings are intentionally accepted, record that decision in
the PR body before merging; do not silently drop `-D warnings`.

An assistant-side native `rustfmt 1.10.0-stable --check --edition 2021` run
(from the Rust 1.99.0 npm toolchain package) over all tracked `.rs` files,
using the repository `rustfmt.toml`, reported no diffs after the fix. The GUI
port, its compile-fix follow-up, the window borrow-check fix, the Clippy
`io_other_error` fix and the consolidated Clippy cleanup were checked with the
same static formatter/parser pass. These are static checks only: the
user-side
`cargo fmt --all -- --check` command above remains the release gate.

The post-download action feature commit, the GUI port commit, the follow-up
compile-fix/warning-cleanup commit, the window borrow-check fix commit, the
Clippy `io_other_error` fix commit and the consolidated Clippy cleanup commit
each changed the SHA again. Do not reuse the earlier `65f5f1e` matrix request
as current; run the eight commands below at the final SHA.

Test-only hooks must not be compiled into a production daemon. After any
hook-enabled build, rebuild with default features before installing or running
the daemon.

The public-network test is ignored by default. Re-run it only if a fresh
network acceptance record is needed:

```sh
RGET_PUBLIC_TORRENT="$HOME/rget-public-multifile/alice_in_wonderland_librivox_archive.torrent" \
RGET_PUBLIC_TIMEOUT_SECS=3600 \
cargo test -p downloader-core --test torrent_public_ipc --locked -- \
  --ignored --nocapture public_multifile_download_and_verify_all_v1_pieces
```

Use the named filter; bare `--ignored` would also try the single-file test.

## 4. Documentation and PR updates completed in this branch

- `README.md`: Linux-only scope, current v1 torrent status, generalized paths,
  finalization link and license section.
- `docs/torrent-phase-five.md`: append-only history note and finalization status.
- `CHANGELOG.md`: pre-release `0.1.0` entry and known limitations.
- `.gitignore`, `LICENSE`, `LICENSE-MIT`, `LICENSE-APACHE`,
  `rust-toolchain.toml`.
- `Cargo.toml`: `librqbit` pinned to `=9.1.0`; `Cargo.lock` already resolves
  `9.1.0`, so no lockfile rewrite is expected.
- PR #1 title and body updated for the final scope and the `--squash` strategy.
- A formatting/Clippy fix commit applies `rustfmt` across the workspace and boxes
  `DaemonEvent::ProgressUpdate`'s `DownloadInfo` payload. Serde framing is
  unchanged; producer, consumer and test constructions were updated together.
- A post-download action feature commit adds a daemon-wide "Open containing
  folder" setting, IPC get/set requests, SQLite persistence, completion
  detection on the progress sampler, a GUI Settings control and an updated
  Post-download actions tab. The setting is global, daemon-side and applies to
  all completed task types; it is skipped on headless daemons without a
  graphical session.
- A GUI port commit replaces the basic `egui` layout with the design in
  `docs/gui-design/index.html`, wires core IPC only, keeps unsupported controls
  disabled with `TODO(state.md)` comments, and adds `state.md` as the living
  backlog. It also exposes a global `SetLimit` control and bulk
  pause/resume/remove-completed fan-out.
- A GUI compile-fix/warning-cleanup commit corrects the Properties dialog's
  `torrent_display` tuple destructure, removes the rustc warnings from the same
  `cargo test --workspace` run (unused imports, unused fields, test-only helpers)
  and keeps `-D warnings` intact.
- A GUI window borrow-check fix commit resolves two `E0499` errors in the Actions
  and Properties windows: `Window::open(&mut open)` is kept out of the `show`
  closure, which now uses a separate `close_clicked` flag.
- A Clippy `io_other_error` fix commit replaces
  `std::io::Error::new(ErrorKind::Other, ...)` with `std::io::Error::other(...)`
  in `crates/cli/src/ipc_client.rs`, after STEP 7 reported `exit=101` at the
  previous SHA.
- A consolidated Clippy cleanup commit clears the remaining 39 unique
  `-D warnings` findings (30 in `downloader-core` lib, 36 in its test target):
  `Default` implementations, boxed `Decision` variants, `checked_div`, `clamp`,
  range `contains`, `contains_key`, `next_back`, explicit `truncate(false)`,
  elided lifetimes, `OsStr` comparison and an MSRV-safe `match` in place of
  the 1.82-only `Option::is_none_or`. `clippy.toml` now sets
  `too-many-arguments-threshold = 12` for the daemon's wide internal
  signatures; this is project lint configuration, not a waiver of
  `-D warnings` or a per-site `#[allow]`. Refactoring those signatures into
  parameter structs is tracked in `state.md`.
- `README.md` and `CHANGELOG.md` describe the ported GUI, wired controls and
  deferred work.

No `.github/workflows` CI was added; validation remains manual on the user's PC.
There are no assistant-side Cargo runs, GUI checks or peer connections.

## 5. Merge gate

Only after the full matrix is green at the final SHA:

```sh
gh pr merge 1 --squash
```

Do not use `--auto`. The merge command must be the last command in its shell
chain. Then verify:

```sh
git fetch origin master
git log --oneline -1 origin/master
gh pr view 1 --json state,mergedAt --jq '{state,mergedAt}'
```

This flow does **not** create a `v0.1.0` tag or GitHub Release. If a public
release is wanted later, complete the manual GUI/live-HTTP checks in
`docs/manual-live-verification.md` and `docs/collision-live-verification.md`,
then add release notes, binary checksums and a tag.

## 6. Current conclusion

The docs/hygiene commit, the formatting/Clippy fix commit, the post-download
action feature commit, the GUI port commit, the compile-fix/warning-cleanup
commit, the window borrow-check fix commit, the Clippy `io_other_error` fix
commit, the consolidated Clippy cleanup commit and PR metadata are prepared.
The merge is blocked on one user-side gate: paste the green results for the full
final matrix at the new SHA. Until then, PR #1 remains open and unmerged. See
`state.md` for the long-term backlog; it is not a merge blocker.
