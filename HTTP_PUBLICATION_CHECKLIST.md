# HTTP publication safety checklist

Scope: confirmed HTTP publication only. Confirmed start, Retry, and Restart from beginning remain rejected. Torrent stage reuse, collision-pause behavior, and GUI work are deferred.

## Drafted, not yet verified with Rust tests

- [ ] Durable SQLite intent primitives record the task, random partial attempt, bound root identity, owned partial inode, and approved destination. Existing intents cannot be overwritten. The primitives are **not wired into publication**.
- [ ] Startup marks a task with any raw HTTP intent as `publication outcome uncertain—inspection required`, including malformed intents and previously completed/canceled snapshots; automatic recovery is skipped.
- [ ] Queue snapshot, checkpoint, partial-reservation, root-binding, and ordinary HTTP removal guards preserve unresolved intent evidence. These are defense in depth, **not a complete lifecycle barrier**.
- [ ] Draft unit tests cover journal identity, simulated link/sync stages, replaced root and partial, uncertain startup, snapshot preservation, and rebinding with an orphaned intent. They have not been compiled or run here.

## Still required before enabling confirmed operations

- [ ] Design and implement a publication/lifecycle barrier with a consistent lock order and durable queue snapshots; serialize cancellation, removal, and re-add with the exact attempt. Audit every database mutation path and cancellation of asynchronous operations.
- [ ] Wire intent creation before every confirmed single-stream and split final link. Link the verified opened inode, not a replaceable source pathname. Preserve no-overwrite collision behavior.
- [ ] Implement crash recovery for before link, after link before directory sync, and after sync before task completion. A journal or matching name, size, content, or inode alone must not incorrectly prove that this operation created a pre-existing destination. Ambiguity must retain all evidence.
- [ ] Handle root moves/replacement throughout publication and recovery. Retain journal and partial and skip cleanup in the moved directory on uncertainty.
- [ ] Design ownership-safe, root-anchored partial and sidecar cleanup, including concurrent entry replacement and root moves. Sidecar inode ownership is not durable today. Until proven safe, retain both.
- [ ] Add real crash-boundary and adversarial concurrency tests for publication, recovery, removal/re-add, collisions, and root movement; test both HTTP paths and unsupported `O_TMPFILE` behavior without fallback.
- [ ] Compile and run targeted core tests and relevant integration tests on a Rust-enabled machine; resolve any failures. No Rust tests for this draft have been run in the sandbox.

## Later, outside this scope

- [ ] Usable confirmed HTTP Retry and explicitly confirmed destructive Restart; durable collision pause/approval.
- [ ] Safe torrent stage reuse and retry, real daemon-backed GUI, Last-Modified resume verification, and YouTube.
