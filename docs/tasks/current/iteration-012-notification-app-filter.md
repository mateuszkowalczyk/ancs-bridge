# Iteration 012 — Notification app filter

**Status:** Planned

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
- [ ] Validate filtered and unfiltered notifications on a real iPhone/desktop session across reconnect.

## Validation

Hardware notification validation remains user-run: configure Messages filtering, restart the daemon, verify Messages Added/Modified notifications stay hidden while an unlisted app delivers, then reconnect the iPhone and repeat. No iteration completion or archive approval has been requested.

Both validation checkpoints passed: formatting, all-target tests (109 passed,
2 hardware tests ignored), all-target/all-feature Clippy with warnings denied,
and debug build. Real `status` CLI smoke checks passed for legacy configuration,
configured filtering, Unicode-control rejection, and the entry-count limit,
including empty stdout and value-free errors on failure.

Separate reviewer subagent `FilterReview` reported no remaining findings and
no worthwhile behavior-preserving simplifications. No behavior tradeoffs were
introduced during simplification review. The existing `bluer 0.17.3` dependency
emits a Rust future-incompatibility warning.
