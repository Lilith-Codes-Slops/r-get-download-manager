# Phase 5: independent groundwork and API probe

> **Status note:** This is an append-only engineering log. Sections below that
> say a feature is “not yet integrated”, “disabled”, or “pending” describe that
> point in the development history. See [Finalization status](#finalization-status)
> for the current release gate.

Requires Phase 1 download/complete and Phase 2 identity/remove. Upload and
seeding remain deferred. Phase 5 proceeds without them. The seed-regression
criterion is deferred until seeding exists. Fast resume remains disabled.
Torrent sessions still set `disable_upload: true`. No independent Phase 5
work here changes the peer-wire/piece pipeline or enables upload.

## Locked engine probe: librqbit 9.1.0

The following is a source inspection, not a runtime verification:

* **Multi-file:** filesystem storage writes files below its selected output
  folder. `Session::get_default_subfolder_for_torrent` selects a subfolder only
  for two or more files: info name, then magnet name, then longest file stem.
  A one-entry multi-file torrent therefore does not automatically get a root
  directory. `AddTorrentOptions::output_folder` can override the folder.
  rGet currently rejects all multi-file metadata.
* **File priorities:** `AddTorrentOptions::only_files` and
  `Session::update_only_files` expose selection, but source inspection found no
  normal/high per-file priority API. Selection is not priority. rGet records
  the validated single-file name, size and normal priority in its task identity;
  it does not apply priorities or enable selection. Multi-file records await
  safe multi-file integration.
* **Webseeds:** no BEP 17/19 payload source or webseed option was found in the
  locked librqbit source. The torrent metadata model does not expose `url-list`
  or `httpseeds`. rGet now counts declared byte strings in original local
  metainfo and logs that they are skipped; the count is not URL validation.
  It deliberately does not log or store potentially sensitive URLs. Magnet
  metadata cannot establish whether outer metainfo webseed declarations exist,
  so their count is unknown. Invalid declarations are reported as unknown and
  do not block P2P downloads. There is no custom HTTP range downloader.
* **Diagnostics:** live snapshots expose accepted fetched bytes, verified
  bytes/piece count, and aggregate peer counts. rGet samples time to first
  verified piece and derives stall episodes itself. The examined snapshot does
  not expose exact duplicate bytes, hash-failure count or a request queue depth;
  these remain unknown. Fetched minus verified is not an exact duplicate count.
* **Transport:** the outgoing connector tries TCP first and, when uTP is
  configured, starts uTP after one second or TCP failure. Listener modes
  include TCP-only, uTP-only and both, with TCP-only the listener default.
  `SessionOptions::bind_device_name` exposes device binding (documented for
  Linux/macOS). `ListenerOptions::enable_upnp_port_forwarding` exposes UPnP;
  it defaults off. No MSE preference/control or NAT-PMP/PCP option was found.
  rGet applies no new bind or NAT setting. Capability logging is version-gated
  and is not evidence that a transport or mapping succeeded.

## Directory publication safety assessment

The existing single-file hard-link publisher cannot publish a directory.
A free-name check followed by ordinary rename is insufficient: another process
may create the destination, and an ordinary rename can replace an existing
empty directory on Unix. A future implementation needs an atomic no-replace
operation, such as Linux `renameat2(RENAME_NOREPLACE)` on the same filesystem,
with a fail-closed path where unavailable. `mkdir` followed by populating the
public directory is not atomic publication and can expose partial contents.

Before starting a download the implementation must validate the root and each
metadata component, reject symlinks, padding and escaping paths, duplicates
and file/directory prefix conflicts, and validate file counts and lengths. It
must also check writability without treating an advisory existence check as a
reservation. Platform-specific names and filesystem aliases need consideration.
The new component/suffix helpers are only preliminary: they do **not** perform
these full checks and are not connected to the add path.

Download into an owned private staging tree on the same filesystem; stop the
engine, verify and sync files and directories, then atomically rename no-replace
with numeric retries. Cancellation/errors before rename should leave no public
partial directory; cleanup must only remove proven-owned staging content, or
retain it explicitly if safe cleanup cannot be established. After a successful
rename, a later directory-sync failure cannot honestly be reported as if no
public directory existed: reconciliation and durability need explicit handling.
Crashes and hostile changes to staging paths need their own recovery tests.
An isolated, not-yet-integrated no-replace rename primitive now targets Linux
GNU via `nix::fcntl::renameat2`. It anchors source and destination operations
to caller-provided directory descriptors, retries collisions, fails closed on
other targets and distinguishes a published-but-unsynced result from an error
before rename. It does not establish source ownership, validate or sync the
tree, prevent hostile source replacement, clean up, or recover after a crash.
The primitive's new tests have not been compiled or run in this environment.
No multi-file publisher is enabled until these cases are implemented and tested.

## Current progress and verification

A completion log summarizes the existing sampled first-piece and stall metrics;
unknown counters stay unknown. Version-gated transport capability logging and
preliminary component/naming helpers are included. Single-file default priority
records and local-metainfo webseed declaration counts passed the user's
workspace and no-default-feature tests. Multi-file priority
storage, safe directory publication and multi-file downloads are not implemented. No Phase 5 exit criterion is claimed complete.

The user reported Phase 4 queue unit tests green; live limit-one promotion and
queued-cancellation tests remain outstanding and can run in parallel. Phase 3
induced stall/recovery and Phase 4 seed/upload verification remain deferred.

## Staging groundwork (not integrated)

An isolated manifest validator rejects empty or unsafe components, duplicate
paths, file/directory prefix conflicts, inconsistent total lengths, overflow,
and excessive file/component/path counts. Legitimate zero-length files are
allowed. This validator does not yet extract paths from librqbit metadata or
check filesystem-specific aliases. The daemon still rejects multi-file torrents.

A Linux staging-area helper creates a private `.rget-staging` parent and fresh
mode-0700 task directories using anchored descriptors and no-follow opens. It
checks parent ownership and permissions, writes a synced ownership marker,
and returns a serializable task/name/device/inode record. Verification
rechecks those values and rejects changed, non-private or symlinked directories
and markers. The record is **not yet persisted by the daemon**, and the helper
is not used by the download path. Setup cleanup removes only an empty directory;
if a marker or unknown content remains, it is left for inspection. This does
not provide recursive cleanup, crash recovery, payload verification or a
complete hostile-same-user isolation guarantee. No rename is called by this
helper. These new tests have not been compiled or run in the sandbox.

The user clarified cleanup precedence: retain staging on verification or
pre-rename sync failure; delete it for other pre-rename failures; after a
successful rename keep the published tree even if destination sync fails.
Remove-from-list retains published and inspection-stage data; remove-and-delete
should delete both. Cleanup must fail closed if ownership cannot be proven:
an ownership-check failure cannot safely authorize deleting an unknown tree.
Crash cleanup requires durable records and recovery; it is not implemented.

An isolated Linux verification helper now walks a `payload` directory beneath
a verified stage using descriptor-relative, no-follow opens. It compares the
full tree against the validated manifest, rejects missing, extra, non-regular,
wrong-size, symlinked, hardlinked or cross-device files, and syncs files and
directories bottom-up. Its caller must first stop and join the engine and
prevent concurrent modifications; it does not hash file contents. Verification
failure does not remove anything. Neither this helper nor the payload creation
helper is integrated into the daemon, and the new tests have not been run in
the sandbox. Multi-file publication remains disabled.

## Metadata preflight groundwork

Local multi-file metainfo is now preflighted from the locked librqbit raw info
model before a staging directory is created. The preflight checks paths,
attributes, checked total length and piece-hash count, rejects lossy or
non-UTF-8 names, and confirms the engine's decoded layout matches. Valid
multi-file requests are still rejected with an explicit message: recording a
manifest in a task, durable ownership and cleanup are not yet integrated.
Magnet metadata remains subject to the existing single-file-only resolver and
may allocate an empty staging directory before rejection; it does not start a
payload download. These changes have not been compiled or run in the sandbox.

The user subsequently reported both workspace and no-default-features runs
passing after the local multi-file metadata preflight increment.

## Durable staging-record groundwork

An isolated SQLite table now supports a reservation before directory creation,
then records the marker-bearing directory's device/inode before payload
creation. Stage names and task IDs are unique, reservations cannot overwrite an
existing attempt, and a verification-failure state is represented separately
from an interrupted stage. Row mutation checks the exact task/name/identity;
unknown, malformed or mismatched records fail closed. The staging helper can
create an exact reserved name and will not adopt an existing one.

The record store is not yet connected to downloads or remove. A conservative
startup recovery pass is now connected; see below. There is no publication
transaction yet. In particular, a
crash after `mkdir` but before inode confirmation leaves a reserved row without
a proven inode; it must not be blindly deleted or adopted. Reconciliation for
that interval and the rename/database crash window must be designed before any
payload or publication integration. The record table and tests have not been
compiled or run in this sandbox. Existing multi-file requests remain rejected.

## Owned-stage cleanup groundwork

An isolated two-pass cleanup helper checks the staging marker and identity,
then traverses payloads relative to directory descriptors. It rejects symlinks,
hardlinks, unexpected root entries, foreign ownership and cross-device entries
before deleting. A separate store wrapper requires an exact durable row; it
keeps retained verification failures unless explicitly requested for deletion,
and leaves the row in place when cleanup reports an error. These paths are not
wired into cancel, remove or shutdown. Startup recovery can clean only the
restricted proven-owned cases below; an unconfirmed reservation cannot
authorize deletion.

The tree must be exclusively controlled after the engine is joined: inode
rechecks cannot eliminate races from a malicious same-user process concurrently
replacing entries. Cleanup can be partial if a later unlink or sync fails. In
particular, if final directory removal fails after marker deletion, the row is
kept but automatic ownership verification may need manual reconciliation.
The tests have not been compiled or run in this sandbox.

## Conservative startup recovery

After acquiring the daemon lock and before accepting tasks, startup scans the
staging table. A `staging` row with a matching ownership record and still-present
payload can be cleaned; the row is removed only after cleanup succeeds.
`retained` rows, unconfirmed reservations, missing payloads, unrecorded trees,
and ownership or cleanup failures are left intact. A missing payload is
ambiguous because a future publisher might have renamed it before recording the
destination. Missing or unsafe staging parents are not created by recovery.

Before the future download path can use this recovery, it MUST durably mark a
stage `retained` before verification begins and keep it that way through
publication: otherwise a crash between failed verification and persisting
retention could incorrectly delete evidence. Publication intent and atomic
queue/record reconciliation are still required to handle the rename-to-database
crash window. No multi-file task creates these rows yet. The recovery pass and
tests have not been compiled or run in this sandbox.

## Publication intent groundwork

An isolated durable intent can record one exact destination plus the source
payload directory's device/inode, conditional on a matching `retained` staging
record. An active intent blocks the recorded-stage cleanup helper. A separate
inspection helper reports whether the recorded payload appears at its source
or at that exact destination; an observation is evidence only, not authority
to delete data or mark a task complete. It must run without concurrent writers.
No automatic intent replay or queue-state reconciliation is implemented. A
future integration must serialize cleanup, intent creation, publication and
queue snapshots under a shared lifecycle gate; the current separate DB checks
alone cannot exclude a concurrent filesystem deletion/intent race.

A new exact-name no-replace rename helper returns an error on a collision; it
never silently advances to a suffix. This distinction matters because the older
helper automatically retries suffixes, which would make a previously persisted
single-name intent inaccurate. A future collision retry must safely reconcile or
replace the previous intent before journaling the next exact candidate. The
new journal and helper are not connected to torrent downloads. Atomic recording
of completion, intent removal and stage-record removal remains outstanding.
The new tests have not been compiled or run in this sandbox.

## Completion transaction groundwork

An isolated completion operation checks that the journaled payload identity is
at the intended destination, then writes the completed task and removes its
exact publication intent in one SQLite transaction. It holds the persistence
snapshot gate and task lock during that operation, so a concurrent queue
snapshot through the same store cannot overwrite it. A publication sync warning
has a backward-compatible optional task field and is displayed by `dl list`.
A database failure leaves the intent, stage record and in-memory task intact.

The staging row deliberately remains after completion until a later verified
cleanup removes the empty owned stage: removing the row first would leave an
unrecorded directory if cleanup or the process failed. The operation is not
connected to downloads or startup intent replay, and callers must serialize it
against all publication and cleanup operations and prevent external mutations.
It does not itself verify the staged tree's contents; that remains a required
pre-rename step. The new tests have not been compiled or run in this sandbox.

## Persisted manifest groundwork

A new optional task-identity `root_name` field and the existing file list can
represent a validated multi-file manifest. An isolated persistence operation
checks the task ID, v1 info-hash and metadata state, then atomically updates
the existing queue record only if there is no staging reservation. Queue
snapshot writes share its persistence gate. Priorities remain stored only.
`dl list` can display the root name and file count when they are present.

This operation is not connected to add or download: valid multi-file requests
are still rejected, and no staging/payload writes are enabled by it. A future
caller must serialize it against reservation and task lifecycle operations and
invoke it only with the validated librqbit-extracted manifest. Automatic
publication-intent recovery and completed-stage cleanup remain unfinished. The
new tests have not been compiled or run in the sandbox.

## Published-tree inspection groundwork

An isolated read-only inspection reconstructs a validated manifest from a
persisted task, checks the exact journaled destination identity, and walks its
layout and lengths without following symlinks. It rechecks the journaled
identity afterwards. It does not adopt the destination, change task state,
delete anything, or prove content hashes: same-size content changes remain
undetectable. It must run under exclusive lifecycle control. Automatic intent
replay is not wired in, and multi-file downloads are still disabled. The new
tests have not been compiled or run in this sandbox.

## Pre-publication verification evidence

An isolated `begin_verified` operation now validates the persisted manifest,
checks and syncs the staged tree, creates an exact-name intent, and records a
fingerprint binding the task metadata to the intent and directory identities.
If writing the evidence fails, the intent remains unproven and cannot be
assumed replayable. Completion removes evidence together with the intent in its
existing SQLite transaction. Evidence can detect changed records; it is not a
MAC, a piece hash, or proof that the engine actually finished. The caller must
still stop/join a successfully completed engine and exclude concurrent writes.

This operation and automatic replay are not connected to multi-file downloads.
No test has been compiled or run in the sandbox for this increment.

An isolated recovery-evidence classifier combines the durable evidence with
identity and layout inspection. Its outcomes are observations only, not
permission to replay, publish, delete, or complete a task. A matching layout
cannot detect same-size edits, and source presence does not rule out a separate
collision at the destination. Automatic reconciliation remains disabled.

## Lifecycle lock groundwork

A process-wide per-task async lock registry is available for future multi-file
operations. Locks for different tasks do not block one another; canceled waiters
cannot replace a live lock. It is **not wired into** transfer, cancel, removal,
or recovery yet and therefore does not currently serialize those paths.
Integration must use the same registry everywhere, include persistence and
filesystem operations in the protected scope, and never wait for a worker while
holding a guard the worker needs to finish. These are process-local locks, not
protection from unrelated processes or hostile same-user filesystem changes.

The isolated pre-stage manifest transaction also rechecks the durable task's
metadata state, source, info-hash, creation identity and existing manifest before
updating it. A stale in-memory task cannot overwrite a changed durable task.
It still requires the shared lifecycle guard, and ingress is not wired to it.

A guarded pre-stage operation now writes the validated manifest and exact stage
reservation in the same SQLite transaction. It returns the process-local task
lock to the caller and performs no filesystem allocation. Failure rolls back
both writes. This is still an isolated primitive: no ingress, allocation, cancel,
removal, or recovery path uses it yet, so it does not enable multi-file torrents.
If the awaiting caller is canceled after the SQLite operation starts, the
blocking operation retains its task and persistence guards until it finishes;
any committed reservation is left for conservative recovery rather than being
silently discarded. This path must be integrated with cancellation-aware
allocation and the existing reservation crash-window rules before use.

The guarded reservation can now allocate a stage from a trusted directory
handle: it checks the reservation, creates a marker-bearing directory without
adopting an existing name, confirms its identity in SQLite, then creates the
payload. The lock stays with the result. Failures clean only when ownership and
record state can be proven; otherwise they leave the reservation or stage for
inspection. If an awaiting caller is aborted, the blocking operation keeps the
lock until it ends; an abandoned successful stage remains recorded for startup
recovery. This allocator is isolated and does not yet start a torrent engine.

An isolated list-only multi-file resolver now preflights local bytes before
librqbit, rechecks the returned bytes and v1 hash, and compares the engine's
file list against the validated manifest. It does not allocate staging or start
payload storage. Magnet input is bounded and selective indices are rejected,
but librqbit's magnet resolver validates received metadata *before* rGet can
preflight its raw bytes; its unchecked length sum remains an unresolved risk.
This resolver is not wired into ingress and magnet resolution has not been
verified end-to-end. Multi-file downloads remain disabled.

An isolated guarded publication sequence now marks the stage retained before
verification, checks and syncs its manifest layout, journals one exact name and
performs no-replace rename against the same opened download root used for stage
allocation. Collisions and unsupported no-replace retain the stage and journal;
post-rename sync failures are returned as warnings, not rename failures. The
caller must have stopped and joined a successfully finished engine and persisted
finalizing state. This operation does not complete the task or clean the stage;
completion, recovery and download ingress remain unwired. Checks are not piece
hash verification and do not protect against concurrent same-user writes.

A guarded published stage can now invoke the isolated completion transaction,
then clean its owned staging record. The transaction rechecks durable identity
and finalizing state and checks that the destination root still matches the
opened allocation root. Rename success is not reversed on a completion error;
the intent remains for reconciliation. Cleanup failure after commit leaves the
completed task and retained stage record, returning a warning. This sequence is
still not connected to torrent downloads or automatic crash reconciliation.

For future crash-time piece verification, an isolated transaction can now
persist the validated exact v1 info dictionary together with the manifest and
stage reservation. It excludes top-level tracker and webseed fields; unknown
info-dictionary fields may still be sensitive. The stored bytes can be
revalidated against the info-hash and manifest, but no payload hash check or
automatic reconciliation is wired yet. Metadata retention/removal policy still
needs integration before this path is used by downloads.

An isolated, descriptor-relative recovery verifier now checks v1 SHA-1 piece
hashes in metadata file order, including cross-file pieces, empty files and a
short final piece. It first validates the stored info-hash and manifest, checks
the exact tree before and after reading, and rejects symlinks and hardlinks.
The caller must establish destination identity and exclude concurrent writes.
This is a content inspection primitive only: it is not wired to startup replay,
does not adopt or delete anything, and does not enable multi-file downloads.

An isolated journaled-destination inspection now combines durable publication
evidence, exact destination identity, bounded stored info-dictionary validation
and the v1 piece verifier, then rechecks identity and layout. Missing metadata is
reported separately and invalid metadata or mismatched pieces fail closed. This
remains observation only: it neither updates tasks nor performs automatic crash
replay, and callers must exclude concurrent filesystem writers.

Startup now attempts a narrow publication crash-window reconciliation before
loading the queue: only an exact journaled destination with valid durable
proof, manifest, info-hash and matching v1 piece hashes is completed. It tries
to sync both rename parents and persists any sync failure as a warning. It
never retries a rename; source-present, missing-data, modified, and ambiguous
cases remain untouched. Completion is atomic with intent removal, followed by
owned-stage cleanup; cleanup errors retain the record. This path assumes the
singleton lock and no concurrent external writers. Multi-file downloads and
full remove/keep/delete integration remain disabled.
A second startup pass can remove an empty, identity-verified retained stage
whose task is already durably completed and has no outstanding publication
intent. It never removes a retained stage that still has a payload.

Completion now records the download root and published directory identities in
its SQLite transaction. An isolated removal primitive checks this durable
record and task identity, then walks and deletes descriptor-relative without
following symlinks or hardlinks. It intentionally leaves the task and identity
records after deleting files: crash-safe removal intent and task deletion are
not implemented, and the primitive is not wired into the user-facing remove
command. It requires exclusive control and does not defend against hostile
same-user races or inode reuse. Existing completions without a published record
are not eligible for this deletion path.

An isolated completed-torrent remove-and-delete operation now durably records a
removal intent before cleaning a proven-owned stage and published directory,
then atomically removes the task and auxiliary ownership records. Its lifecycle,
persistence and task guards survive cancellation of the waiting caller. It is
not connected to the remove command: startup replay and reconciliation for the
crash window after filesystem deletion are still missing. On any error after
intent creation it retains the task and intent rather than guessing that a
missing directory was deleted by rGet. Remove-from-list/keep-files also remains
to be integrated.

Startup can now resume a journaled completed-torrent removal only when the
stored task, intent, published identity and existing directory still match. A
missing or changed destination remains unresolved; it is not treated as proof
of successful deletion. A separate isolated keep-files transaction removes a
stopped task from the list while retaining its files and marking a detached
staging row retained. Neither operation is connected to the IPC remove command.

The remove command now routes stopped, recorded multi-file tasks through the
new keep/delete operations; legacy HTTP and single-file torrent removal retains
its prior path. It refuses active tracked tasks and does not yet delete failed
or canceled multi-file stages through IPC. Multi-file downloads remain disabled,
and lifecycle integration must still ensure a future active worker is stopped
before removal. Startup removal replay remains conservative for missing paths.

The isolated multi-file resolver now rejects magnets before calling librqbit.
In locked librqbit v9.1.0, the raw peer metadata reader and its connector are
private, while the public list-only magnet API validates first and computes an
unchecked length sum. No supported pre-validation hook was found. Local
metainfo preflight remains available; multi-file download ingress is still off.

The locked librqbit `Session::stop()` pauses/cancels and sleeps for one second;
it does not expose a join of all spawned engine tasks. An isolated storage gate
and wrapper around its default filesystem storage now provide a potential
quiescence barrier: close waits for admitted storage calls and rejects later
calls, including calls on storage returned by `take`. The wrapper delegates
writes unchanged and does not implement a picker or assembler. It is not wired
into downloads, has not been proven with a live engine, and does not itself stop
or join engine tasks. Local multi-file ingress remains disabled until the full
lifecycle is integrated and this behavior is validated.

A local librqbit-to-librqbit smoke test exercises the delegated filesystem
storage, stops the downloader, closes the gate, and checks the resulting v1
multi-file pieces. It uses direct localhost peers, not a tracker, and does not
exercise daemon ingress, cancellation, crashes, or prove all engine tasks joined.

The daemon now routes validated local multi-file metainfo through durable
manifest/reservation, owned staging, librqbit's gated filesystem storage,
storage quiescence, exact no-replace publication and transactional completion.
The user approved the storage barrier as a substitute for an engine-task join;
`Session::stop()` still does not promise those tasks have joined. Multi-file
magnets remain disabled. Cancel and remove signal and wait without the lifecycle
guard; stopped unpublished tasks can delete only verified-owned stages, while
journaled or ambiguous cases fail closed. This path and its new local-tracker
IPC tests have not yet been compiled or run in this environment; the public
multi-file acceptance test is still outstanding.

The live local multi-file path also rechecks v1 piece hashes from the durable
info dictionary after storage quiescence and before journaling or rename. The
stage is marked retained before this check, so a mismatch leaves it for
inspection. This is distinct from librqbit's download-time verification and
still assumes exclusive control of the staging tree during verification and
publication. The added integration and mismatch test need a fresh test run.

The opt-in `torrent-test-hooks` feature enables daemon fault-injection markers
used only by `torrent_fault_ipc`: crash after rename and durable completion,
post-barrier corruption and missing-file checks, collision and root replacement,
interrupted delete/keep transactions, concurrent tasks and cancellation. The
late-write probe uses the live gate and the actual delegated-storage wrapper with
a sentinel delegate; it is not proof that engine tasks have joined. Local test
seeders serve the fixtures, while the daemon remains download-only. These hooks
are not compiled into normal builds and must not be enabled in production.
Run the dedicated tests serially:

```sh
cargo test -p downloader-core --features torrent-test-hooks --test torrent_fault_ipc --locked -- --test-threads=1
```

Acceptance review: a failed collision can leave a source-present publication
intent. Keep/delete now abandon that intent only for a stopped unpublished task
whose durable identity still matches and whose exact staged payload is proven
present; a destination match or ambiguous observation still fails closed. This
keeps the existing destination untouched. The GUI status now includes the
validated root and file count when available.

A public-network check is available as the ignored `torrent_public_ipc` test.
Set `RGET_PUBLIC_TORRENT` to an existing local v1 multi-file `.torrent` with
reachable peers, then run the ignored test with `--nocapture`. It creates an
isolated temporary daemon home, displays the expected root/count, downloads,
checks the reported identity, and makes a second v1 piece-verification pass
using the project's recovery verifier. On success it removes the isolated test
home; on failure it prints and leaves that directory for inspection. Network
availability, remote seeders and any metainfo size or feature restrictions can
prevent completion. Public-network acceptance is still pending until it passes.
If the storage barrier itself fails or its blocking worker panics, the task is
now marked `quiescence-unknown`, even when cancel was requested. Both removal
modes refuse while that daemon is running; a restart ends its engine tasks and
then applies conservative durable recovery. This is not a task-join guarantee.
The fault suite now forces two active torrent slots and clears inherited test
hook variables for each isolated daemon, so a shell exporting
`RGET_TORRENT_MAX_ACTIVE=1` cannot serialize the concurrent-barrier test. The
public acceptance test likewise isolates its daemon and rejects single-file,
one-entry and v2/hybrid metainfo before contacting the network. An ordinary ISO
torrent with `length` and `name` is single-file even when its release page
contains several download options.

An additional ignored `public_singlefile_download_and_verify_all_v1_pieces`
test accepts `RGET_PUBLIC_SINGLE_TORRENT`. It downloads an ordinary v1
single-file torrent, checks the recorded file metadata and makes a direct
single-file v1 piece-hash pass. It is a real-world P2P smoke test only; it does
not satisfy the separately required multi-file acceptance run, whose test
continues to require `RGET_PUBLIC_TORRENT` and at least two files.

For the optional single-file smoke test, set `RGET_PUBLIC_SINGLE_TORRENT` to a
local v1 `.torrent` and run only its named ignored test, for example:

```sh
RGET_PUBLIC_SINGLE_TORRENT="/path/to/file.torrent" RGET_PUBLIC_TIMEOUT_SECS=3600 \
  cargo test -p downloader-core --test torrent_public_ipc --locked -- \
  --ignored --nocapture public_singlefile_download_and_verify_all_v1_pieces
```

Its isolated test home holds the complete download, so allow enough temporary
disk space. Webseed declarations are not used; reachable BitTorrent peers are
still required.

## Finalization status

The earlier sections of this document are historical development notes; they are
not a current feature matrix. The daemon now routes validated local v1
multi-file metainfo through staged verification, storage quiescence, exact
no-replace publication, durable identity and conservative recovery. Multi-file
magnets, v2/hybrid/private metainfo, padding/symlink/special files, webseed
retrieval, upload and seeding remain out of scope.

The user reports that the ignored public v1 multi-file acceptance test passed on
their PC, including the post-publication v1 piece-verification pass. A verbatim
final log was **not attached** to this branch, so this is recorded as a
**user-reported pass**, not as an assistant-observed result. In particular, do
not quote `test result: ok` or the exact `completed: ... all v1 pieces
verified` line as text that was captured here. The supplied candidate was the
Internet Archive Alice in Wonderland LibriVox `_archive.torrent`, info-hash
`603f7afe0174288bfee17e41399d066952d5251d`; tracker and peer availability can
change. The earlier CachyOS single-file smoke run does not satisfy this
multi-file acceptance criterion.

The deterministic Cargo suites and public torrent run were reported green on the
user's PC. `cargo fmt --check`, `cargo clippy -- -D warnings` and the release
build at the final SHA are still outstanding. Because the finalization hygiene
commit changes the SHA, the full matrix must be rerun at that new SHA. See
[finalization.md](finalization.md) for the exact commands and merge gate.

PR #1 remains open. The selected merge strategy is `--squash`, but no merge is
permitted until the final matrix is green. No `v0.1.0` tag or GitHub Release is
created in this flow.

## Formatting and Clippy finalization fix

The first full-matrix attempt at `07fd541` reported `cargo fmt --all -- --check`
diffs and one `large_enum_variant` error for
`DaemonEvent::ProgressUpdate { task_id: String, task: DownloadInfo }`. A later
commit applies `rustfmt` across the workspace and changes that payload to
`Box<DownloadInfo>`. Serde still emits and accepts the same JSON event shape;
the daemon publisher, GUI cache, IPC integration test and protocol roundtrip
test were updated together. This is a source-level fix, not a waiver of
`-D warnings` and not a completed user-side matrix run. The final SHA changed
again, so repeat the entire matrix in `docs/finalization.md` before merging.
