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
  normal/high per-file priority API. Selection is not priority. rGet does not
  yet store per-file priorities or enable selection.
* **Webseeds:** no BEP 17/19 payload source or webseed option was found in the
  locked librqbit source. The torrent metadata model does not expose `url-list`
  or `httpseeds`. rGet does not currently extract or count those URLs; it must
  not report that it has enabled or even detected them. There is no custom
  HTTP range downloader.
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
No multi-file publisher is enabled until these cases are implemented and tested.

## Current progress and verification

A completion log summarizes the existing sampled first-piece and stall metrics;
unknown counters stay unknown. Version-gated transport capability logging and
preliminary component/naming helpers are included. File priority storage,
webseed detection, safe directory publication and multi-file downloads are
not implemented. No Phase 5 exit criterion is claimed complete.

The user reported Phase 4 queue unit tests green; live limit-one promotion and
queued-cancellation tests remain outstanding and can run in parallel. Phase 3
induced stall/recovery and Phase 4 seed/upload verification remain deferred.
