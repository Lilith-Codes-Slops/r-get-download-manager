# Finalization status and merge gate

This document records the decisions made for the finalization flowchart, the
completed merges of PR #1 and PR #2, and the verification status of the
post-merge GUI-only slice and its docs-only follow-up. It is deliberately
conservative: a public-network pass is reported by the user, but a verbatim
final log was not attached to this branch.

## Recorded decisions

| Node | Decision |
| --- | --- |
| Public multi-file acceptance | User-reported pass; verbatim final log not attached. Do not quote `test result: ok` as an observed line. |
| Earlier single-file smoke run | Not multi-file acceptance. Close it out and clean its isolated files on the user's PC. |
| Final matrix status | PR #1: nine-step matrix green at `6cc0baf…`, merged as `99c845f…`. PR #2: six-step matrix green at `8bd754e…`, merged as `524a35b…`. Both on `master`. |
| Scope | Merge PR #1 and PR #2 only; both done. No `v0.1.0` tag, GitHub Release or checksums were created. |
| Merge strategy | Executed with `--squash` for both PRs; PR #1 and PR #2 are `MERGED`. |
| Current action | Verify the docs-only follow-up on `arena/af64881e-r-get-download-manager` at its new SHA; the six-step user-side list is below and in `state.md`. |

The nine-step deterministic matrix was rerun at `6cc0baf…` and confirmed green,
then `gh pr merge 1 --squash` produced merge commit
`99c845fd3f496008b2a585ae7592671f332eba68`. A later six-step matrix was green at
`8bd754e…` before `gh pr merge 2 --squash` produced merge commit
`524a35b8fd5660970c98af0b9de302e36c12c390`. Earlier results at `251cb71`,
`70c56e5`, `07fd541`, `65f5f1e`, the post-download feature commit, the GUI port
commit and the follow-up fix commits are useful history but are not the current
verification gate.

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

## 3. Historical matrices and current verification list

The first block below was run on the user's PC from the repository root and was
green at `6cc0baf…` immediately before PR #1 was squash-merged. It is recorded
as evidence, not as a pending gate. The GUI-only follow-up used the shorter
six-step list in `state.md` and was green at `8bd754e…` before PR #2 was merged.

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

A second, shorter matrix was green at
`8bd754e91be1b941b6a74f4430561e2a6d627550` before PR #2 was squash-merged as
`524a35b…`:

```sh
cargo check -p downloader-gui --locked
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --locked -j 2
git status --short; git rev-parse HEAD; /bin/df -h "$HOME"
```

All six steps exited 0; STEP 0 and STEP 8 produced no warnings; STEP 7 needed
no `-D warnings` waiver. The dependency guard against `99c845f…` printed
nothing. The PR #2 body is the authoritative green record. No tag, Release,
checksums or CI workflow were created.

The docs-only follow-up on `arena/af64881e-r-get-download-manager` changes the
SHA without code changes, so rerun exactly these six commands at its new SHA
after `git pull --ff-only origin arena/af64881e-r-get-download-manager`; use
`524a35b…` as the dependency-guard base. Old matrix steps 2–5 remain optional
full-parity checks.

If the release build is memory-heavy, rerun it with `-j 2` (or
`CARGO_BUILD_JOBS=2`); this workspace links `wgpu`, `egui` and `librqbit`. If
`df` is aliased to `duf` on your shell, use `/bin/df -h "$HOME"` to check disk.

At `6cc0baf…` `git status --short` was empty and every command above exited 0;
no Clippy warnings were waived and `-D warnings` was not dropped. The release
build used `-j 2` after an earlier memory-heavy attempt.

An assistant-side native `rustfmt 1.10.0-stable --check --edition 2021` run
(from the Rust 1.99.0 npm toolchain package) over all tracked `.rs` files,
using the repository `rustfmt.toml`, reported no diffs after the fix. The GUI
port, its compile-fix follow-up, the window borrow-check fix, the Clippy
`io_other_error` fix, the consolidated Clippy cleanup and the integration-test
`unnecessary_unwrap` fix were checked with the same static formatter/parser
pass. These were static checks only; the user-side `cargo fmt --all -- --check`
command above was the release gate and passed at `6cc0baf…`.

The integration-test `unnecessary_unwrap` fix was the last pre-merge change; the
matrix above was rerun at that exact SHA (`6cc0baf…`) and the PR body was updated
before `gh pr merge 1 --squash`. No further pre-merge rerun was needed.

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
- A follow-up commit replaces two `partial_name.unwrap()` calls in
  `crates/core/tests/task_actions_ipc.rs` with nested `if let` bindings. A
  warning-only `cargo clippy --workspace --all-targets --locked` run at the
  previous SHA reported those two `clippy::unnecessary_unwrap` warnings and no
  others, confirming the complete lint inventory for all test targets.
- `README.md` and `CHANGELOG.md` describe the ported GUI, wired controls and
  deferred work.
- A post-merge GUI-only commit adds local preference persistence and the `Ctrl+K`
  command palette, extends the Help shortcut list, and removes stale open-PR and
  pre-merge gate wording from `state.md`, `docs/finalization.md`,
  `CHANGELOG.md` and `README.md`.
- A docs-only follow-up commit records PR #2 merged as `524a35b…`, marks the
  six-step matrix green at `8bd754e…`, and updates `state.md`,
  `docs/finalization.md`, `CHANGELOG.md` and `README.md`. It contains no code
  changes.

No `.github/workflows` CI was added; validation remains manual on the user's PC.
There are no assistant-side Cargo runs, GUI checks or peer connections.

## 5. Merges completed

`gh pr merge 1 --squash` was executed after the green nine-step matrix at
`6cc0baf…`, and `gh pr merge 2 --squash` after the green six-step matrix at
`8bd754e…`. Verified state:

- PR #1: `state: MERGED`, `closed: true`, `baseRefName: master`,
  `headRefOid: 6cc0baf7766142d4b3ab7daff0672736650c3129`,
  `mergeCommit: 99c845fd3f496008b2a585ae7592671f332eba68`,
  `mergedAt: 2026-11-11T00:34:09Z`, `mergedBy: Lilith-Codes-Slops`.
- PR #2: `state: MERGED`, `closed: true`, `baseRefName: master`,
  `headRefOid: 8bd754e91be1b941b6a74f4430561e2a6d627550`,
  `mergeCommit: 524a35b8fd5660970c98af0b9de302e36c12c390`,
  `mergedAt: 2026-11-11T08:29:57Z`, `mergedBy: Lilith-Codes-Slops`.
- `git ls-remote origin refs/heads/master` = `524a35b…` after PR #2.
- `statusCheckRollup: []` for both PRs; no CI workflow was added.

No `--auto`, `--delete-branch`, tag, GitHub Release or checksums were created.
Both merge commands were the last command in their shell chain with an explicit
PR number. If a public release is wanted later, complete the manual GUI/live-HTTP
checks in `docs/manual-live-verification.md` and
`docs/collision-live-verification.md`, then add release notes, binary checksums
and a tag.

## 6. Current conclusion

PR #1 and PR #2 are merged; both pre-merge matrices are closed out and green.
The active branch `arena/af64881e-r-get-download-manager` carries only
documentation updates that record those merges and the green six-step record at
`8bd754e…`. No code changed in this follow-up, but the commit changes the SHA,
so its own six-step user-side verification at the new SHA is the only outstanding
gate; see `state.md`. No tag, Release, checksums or CI workflow exist. The
long-term backlog is not a merge blocker.
