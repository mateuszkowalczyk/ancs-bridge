# Iteration 012 — Notification app filter

**Status:** Completed

## Sources

- GitHub issue #1: https://github.com/mateuszkowalczyk/ancs-bridge/issues/1
- `docs/specs/runtime-machine-api.md`
- `docs/specs/ancs-session-forwarding.md`
- User-approved specification additions and implementation improvements.

## Tasks

- [x] Add optional bounded, validated notification policy and preserve it during setup repair/re-pair.
- [x] Filter exact bundle identifiers before app lookup and desktop delivery across notification and session lifecycles.
- [x] Cover validation boundaries, policy persistence, ignored lifecycle, and unaffected delivery with automated checks.
- [x] Document configuration, restart requirements, delivery-only filtering, and downgrade behavior.
- [x] Run automated validation, simplify, and obtain separate subagent review; resolve actionable findings.
- [x] Validate filtered and unfiltered notifications on a real iPhone/desktop session across reconnect.

## Validation

Manual validation passed on 2026-11-13 using the feature build and the user's
paired iPhone/desktop session. The user confirmed TickTick delivered to both
devices without exclusions, then appeared only on the iPhone with
`com.TickTick.task` excluded while an unfiltered control delivered to both.
After an iPhone Bluetooth off/on cycle, the same daemon returned to `ready`
without restart or re-pair; the user confirmed the same filtering/control
results for fresh notifications. TickTick same-UID Modified behavior was not
directly exercised manually; deterministic session tests cover that path.
The user explicitly approved completion and archival on 2026-11-13 after the
readiness report. Fresh completion checks passed: formatting, 109 tests,
all-target/all-feature Clippy with warnings denied, and debug build.

Both validation checkpoints passed: formatting, all-target tests (109 passed,
2 hardware tests ignored), all-target/all-feature Clippy with warnings denied,
and debug build. Real `status` CLI smoke checks passed for legacy configuration,
configured filtering, Unicode-control rejection, and the entry-count limit,
including empty stdout and value-free errors on failure.

Separate reviewer subagent `FilterReview` reported no remaining findings and
no worthwhile behavior-preserving simplifications. No behavior tradeoffs were
introduced during simplification review. The existing `bluer 0.17.3` dependency
emits a Rust future-incompatibility warning.
