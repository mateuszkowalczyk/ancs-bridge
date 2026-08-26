# Iteration 010 — live BlueZ device name in status

**Status:** Completed

## Sources

- `docs/prd/02-machine-api.md`
- `docs/specs/runtime-machine-api.md`

## Tasks

- [x] Resolve the configured phone's current BlueZ alias/name during daemon
  reconciliation and propagate it to the persisted machine status, retaining
  the configured name as a fallback when the live value is unavailable.
- [x] Keep status freshness matching based on stable adapter/device identity,
  not the mutable display name.
- [x] Update machine API documentation, fixtures, and regression tests, then
  run the complete automated validation gate without committing changes.

## Implementation notes

- `deviceName` now reports BlueZ `Device1.Alias`, which BlueZ supplies from the
  current remote name when no custom alias is set.
- The published v0.1.0 tag and release were not changed.
- Formatting, warnings-denied Clippy, all non-hardware tests, a locked release
  build, and `git diff --check` pass. `cargo audit` still reports the existing
  target-inactive `quick-xml` advisories documented for this Linux-only build.
