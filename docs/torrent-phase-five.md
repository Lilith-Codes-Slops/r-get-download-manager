# Phase 5: independent groundwork and API probe

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
