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

These functions are **not connected** to daemon startup, downloads, or remove.
There is no recursive cleanup or publication transaction yet. In particular, a
crash after `mkdir` but before inode confirmation leaves a reserved row without
a proven inode; it must not be blindly deleted or adopted. Reconciliation for
that interval and the rename/database crash window must be designed before any
payload or publication integration. The record table and tests have not been
compiled or run in this sandbox. Existing multi-file requests remain rejected.
