# Uni5DB Bytes maintenance branch

Official upstream: https://github.com/tokio-rs/bytes
Maintenance fork: git@github.com:chenyangzhi88/bytes.git
Maintenance branch: uni5db-1.12.1
Baseline: v1.12.1, commit 76c0fbb54ed4336caf9d2311658a2f4a5627c21d.

The baseline crate is bytes 1.12.1 (MIT), SHA256
fc652a48c352aef3ea3aed32080501cf3ef6ed5da78602a020c991775b0aff04.

Local production changes add `Bytes::from_owner_with_release` and its exact
allocation-layout accessor. The release callback follows destruction of the
pinned owner and deallocation of its controller, including final clone/slice
retirement and ordinary unwind paths. Existing APIs and vtables remain.
Maintained unit tests live in src/tests; original upstream integration tests,
manifest dependencies, CI and benches remain in this complete Git repository.

uni5db-kv-engine consumes this repository as third_party/bytes. Its gitlink pins
a concrete maintenance commit; the branch setting does not automatically upgrade
production. Parent consumers patch Cargo to that same canonical submodule path.

To evaluate an upstream update, add/fetch the official upstream remote, merge an
explicit release/commit into a maintenance branch, review actual source changes,
run owning/upstream tests and affected kernel regressions, then update the kernel
gitlink. Publish the required maintenance commit before sharing that gitlink.
Do not reset away local ownership/release fixes or silently follow upstream HEAD.

## Complete retained allocation capacity

`BytesMut::backing_capacity()` reports the capacity of the retained Vec,
including prefixes and suffixes outside the current view. The Vec representation
uses its stored offset plus visible capacity; the shared representation reads the
existing immutable Vec metadata. The method is read-only and adds no storage or
allocation. `capacity()` retains its original writable-view semantics. Private
regressions cover split, advance, clear, reclaim, shared relocation and retained
aliases. This allows bounded consumers to reason about real retention without
tracking pointer identities or historical allocation estimates.
